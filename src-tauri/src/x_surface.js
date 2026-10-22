// Presentation adapter for a real, interactive X document. No feed scraping,
// credential access, private API calls, or simulated social account actions.
(() => {
  const config = __ATSUMI_X_SURFACE__;
  if (window !== window.top || location.protocol !== 'https:' || !['x.com','www.x.com'].includes(location.hostname)) return;
  if (window.__atsumiXSurface) return;
  let visible = false, pending = 0, lastStatus = '', menu = null, applied = false, verifiedOwner = null;
  let automaticPlayback = true, mediaIntent = null, mediaIntentUntil = 0, scrollMediaIntent = null;
  let visibilityEpoch = 0;
  let pointerRoot = null, pointerAt = 0, routingProbes = 0;
  let routeIntent = null;
  let routeRecovery = null, recoveryEvent = null, historyRevision = 0, inputRevision = 0;
  let watchedAccount = null, accountObserver = null, accountLoaded = null;
  let photoProbeRoute = '', photoProbeEpoch = 0, photoProbeTimers = [];
  const revealed = new WeakSet();
  const photoSecurity = 'input[type="password"],input[autocomplete="one-time-code"],[data-testid="ocfEnterTextTextInput"],[data-testid="LoginForm_Login_Button"],a[href^="/i/flow/"],a[href="/login"]';
  const signal = (kind, value = '') => {
    if (!visible && kind !== 'status') return;
    // A cancelled location.assign still fires WebView2 NavigationCompleted.
    // Use the closed native message bridge; never disturb source navigation.
    try {
      window.chrome?.webview?.postMessage(`ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/${config.nonce}?kind=${encodeURIComponent(kind)}&value=${encodeURIComponent(value)}`);
    } catch {} // No navigation or general IPC fallback.
  };
  const local = raw => {
    if (typeof raw !== 'string' || !raw) return null;
    try { const url = new URL(raw, location.origin); return url.protocol === 'https:' && ['x.com','www.x.com'].includes(url.hostname) && !url.port && !url.username && !url.password ? url : null; } catch { return null; }
  };
  const externalLink = raw => {
    if (typeof raw !== 'string' || !raw || raw.length > 2048 || /[\s\\\u0000-\u001f\u007f]/.test(raw)) return null;
    try {
      const url = new URL(raw, location.origin);
      if (!['https:','http:'].includes(url.protocol) || !url.hostname || url.username || url.password
        || ['x.com','www.x.com'].includes(url.hostname.replace(/\.$/,''))) return null;
      // Keep the ordinary bridge bounded, including URL-encoded query data.
      const packet = `https://x.com/__atsumi_surface__/${config.nonce}?kind=externalLink&value=${encodeURIComponent(url.href)}`;
      return url.href.length <= 2048 && packet.length <= 4096 ? url.href : null;
    } catch { return null; }
  };
  const postPath = raw => {
    const url = local(raw);
    return url && /^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/[1-9]\d{4,24}(?:\/(?:photo|video)\/[1-4])?$/.test(url.pathname) ? url.pathname : null;
  };
  const reservedProfiles = new Set(['home','explore','search','notifications','messages','i','compose','settings','login','logout','signup','tos','privacy','help','about','download','jobs','communities','premium','grok','hashtag','intent','account','accounts']);
  const navigationPath = raw => {
    const post = postPath(raw);
    if (post) return post;
    const url = local(raw), match = /^\/([A-Za-z0-9_]{1,15})$/.exec(url?.pathname || '');
    return match && !reservedProfiles.has(match[1].toLowerCase()) ? url.pathname : null;
  };
  // Linkless quotes/cards are their own interaction scope. A missing quote
  // permalink must never fall through to its surrounding post's identity.
  const nestedRoot = (element, root) => {
    for (let node = element; node && node !== root; node = node.parentElement) {
      if (node.matches('[role="link"]:not(a),[role="button"]:not(button),[data-testid="quoteTweet"],[data-testid="quotedTweet"]')) return node;
    }
    return null;
  };
  const quoteAuthors = (user, root, article) => {
    const own = node => node.closest('article') === article && !nestedRoot(node, root);
    const links = [...user.querySelectorAll('a[href]')].filter(own);
    // An explicit source link remains authoritative. Never replace conflicting
    // or unknown link evidence with a more convenient display-name token.
    if (links.length) return new Set(links.map(anchor => /^\/([A-Za-z0-9_]{1,15})$/.exec(navigationPath(anchor.getAttribute('href')) || '')?.[1]?.toLowerCase()).filter(Boolean));
    const nodes = [user, ...user.querySelectorAll('*')];
    if (nodes.length > 128) return new Set();
    const excluded = 'script,style,template,input,textarea,select,option,[hidden],[aria-hidden="true"],[inert],[role="link"]:not(a),[role="button"]:not(button),[data-testid="quoteTweet"],[data-testid="quotedTweet"],[data-testid="User-Name"]';
    // X's linkless quoted heading still renders a literal @handle. Only a
    // complete own text element is evidence; do not scan post bodies or parent
    // authors, and do not read hidden/nested cards inside the heading.
    return new Set(nodes.filter(node => own(node) && node.closest('[data-testid="User-Name"]') === user
      && !node.closest('[hidden],[aria-hidden="true"],[inert]') && !node.matches('script,style,template,input,textarea,select,option') && !node.querySelector(excluded))
      .map(node => /^@([A-Za-z0-9_]{1,15})$/.exec((node.textContent || '').trim())?.[1]?.toLowerCase()).filter(Boolean));
  };
  const articlePath = element => {
    const article = element?.closest('article[data-testid="tweet"]');
    if (!article) return null;
    const root = nestedRoot(element, article) || article;
    const user = [...root.querySelectorAll('[data-testid="User-Name"]')].find(node => node.closest('article') === article && !nestedRoot(node, root));
    if (!user) return null;
    const paths = new Set([...user.querySelectorAll('time')].filter(node => !nestedRoot(node, root)).map(node => postPath(node.closest('a')?.getAttribute('href'))).filter(Boolean));
    if (paths.size) return paths.size === 1 ? [...paths][0] : null;
    if (root === article) return null;
    // Linkless native quote cards can still expose their own exact photo URL.
    // Join only within this quote, with one explicit matching author. Never
    // borrow an enclosing post/child quote or intercept the photo itself.
    const authors = quoteAuthors(user, root, article);
    if (authors.size !== 1) return null;
    const mediaPaths = new Set([...root.querySelectorAll('a[href]')].filter(anchor => anchor.closest('article') === article
      && !nestedRoot(anchor, root) && anchor.querySelector('img,[data-testid="tweetPhoto"]'))
      .map(anchor => postPath(anchor.getAttribute('href'))).filter(path => path && /\/photo\/[1-4]$/.test(path))
      .map(path => path.replace(/\/photo\/[1-4]$/, '')));
    if (mediaPaths.size !== 1) return null;
    const mediaPath = [...mediaPaths][0];
    return authors.has(mediaPath.split('/')[1].toLowerCase()) ? mediaPath : null;
  };
  const cancelRouteIntent = () => {
    if (!routeIntent) return;
    routeIntent = null;
    signal('routeIntent','cancel');
  };
  const routeOwnerCurrent = () => {
    if (verifiedOwner!==config.owner || document.querySelector(photoSecurity)) return false;
    const profiles=[...document.querySelectorAll('[data-testid="AppTabBar_Profile_Link"]')];
    return profiles.length<=8 && profiles.every(node =>
      /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(local(node.getAttribute('href'))?.pathname || '')?.[1]?.toLowerCase()===config.owner);
  };
  const cancelRouteRecovery = () => { routeRecovery = null; };
  const recoveryCurrent = recovery => routeRecovery === recovery && visible && !document.hidden
    && recovery.epoch === visibilityEpoch && recovery.href === location.href
    && recovery.history === historyRevision && recovery.input === inputRevision
    && performance.now() <= recovery.until && routeOwnerCurrent();
  const recoveryScroll = article => {
    const scrolls = [], page = document.scrollingElement;
    if (page) scrolls.push({node:page,top:page.scrollTop,left:page.scrollLeft});
    for (let node=article.parentElement,depth=0;node && depth<64;node=node.parentElement,depth++) {
      if (node === page) continue;
      const css = getComputedStyle(node);
      if ((/^(auto|scroll|overlay)$/.test(css.overflowY) && node.scrollHeight > node.clientHeight)
        || (/^(auto|scroll|overlay)$/.test(css.overflowX) && node.scrollWidth > node.clientWidth)) {
        if (scrolls.length >= 16) return [];
        scrolls.push({node,top:node.scrollTop,left:node.scrollLeft});
      }
      if (depth === 63 && node.parentElement) return [];
    }
    return scrolls.filter(value=>Number.isFinite(value.top) && Number.isFinite(value.left));
  };
  const recoveryPresented = element => {
    for (let node=element,depth=0;node && depth<64;node=node.parentElement,depth++) {
      const css=getComputedStyle(node);
      if (node.hidden || node.hasAttribute('inert') || node.getAttribute('aria-hidden')==='true'
        || css.display==='none' || css.visibility==='hidden' || css.visibility==='collapse' || css.opacity==='0') return false;
      if (depth===63 && node.parentElement) return false;
    }
    return true;
  };
  const recoveryContent = recovery => {
    // Recreated source nodes are acceptable only when the original outer post
    // is observed again as its own article, not as the destination's quote.
    const roots = [...document.querySelectorAll('[data-testid="primaryColumn"]')];
    if (roots.length !== 1) return false;
    const primary = roots[0];
    const own = article => article.closest('[data-testid="primaryColumn"]') === primary
      && !article.parentElement?.closest('article') && recoveryPresented(article);
    if (recovery.article.isConnected && own(recovery.article)
      && (!recovery.post || articlePath(recovery.article) === recovery.post)) return true;
    if (!recovery.post) return false;
    const articles = [...primary.querySelectorAll('article[data-testid="tweet"]')];
    return articles.length <= 64 && articles.some(article => own(article) && articlePath(article) === recovery.post);
  };
  const recoverSourceRoute = intent => {
    if (typeof window.dispatchEvent !== 'function' || typeof PopStateEvent !== 'function') return;
    let state;
    try { state = window.history.state; } catch { return; }
    // Keep opaque browser state only in this short-lived closure. Never inspect,
    // serialize, persist or send it through the native message bridge.
    const recovery = {href:location.href,state,epoch:visibilityEpoch,history:historyRevision,
      input:inputRevision,until:performance.now()+1000,article:intent.article,
      post:articlePath(intent.article),scrolls:recoveryScroll(intent.article)};
    routeRecovery = recovery;
    setTimeout(()=>{
      if (!recoveryCurrent(recovery)) { if(routeRecovery===recovery)cancelRouteRecovery(); return; }
      // A source router may set its own store after pushState returns. Wait for
      // that handler to finish, then ask its public history listener to reread
      // the unchanged entry once. This is not a real history traversal.
      recoveryEvent = new PopStateEvent('popstate',{state:recovery.state});
      try { window.dispatchEvent(recoveryEvent); } catch { cancelRouteRecovery(); }
      finally { recoveryEvent = null; }
      const restore = attempt => {
        if (!recoveryCurrent(recovery)) { if(routeRecovery===recovery)cancelRouteRecovery(); return; }
        if (recoveryContent(recovery)) {
          for (const value of recovery.scrolls) {
            if (!value.node.isConnected) continue;
            if (value.node.scrollTop !== value.top) value.node.scrollTop = value.top;
            if (value.node.scrollLeft !== value.left) value.node.scrollLeft = value.left;
          }
          cancelRouteRecovery();
        } else if (attempt < 3) {
          // Bounded, read-only paint checks; never repeat popstate or navigate.
          setTimeout(()=>restore(attempt+1),[50,150,300][attempt]);
        } else cancelRouteRecovery();
      };
      setTimeout(()=>restore(0),0);
    },0);
  };
  const interruptRecovery = event => {
    if (!event.isTrusted) return;
    scrollMediaIntent = null;
    inputRevision++;
    cancelRouteRecovery();
  };
  // Capture on window too: Atsumi-owned menu controls stop propagation at the
  // document, but a new real interaction must still revoke pending restoration.
  for (const name of ['pointerdown','keydown','wheel','touchstart']) {
    window.addEventListener(name,interruptRecovery,{capture:true,passive:true});
  }
  // Some official quoted cards have only role=link and an onClick router;
  // there is no canonical href to read. Observe the source's next exact
  // navigation, never infer a status id from the parent or recreate a click.
  const armRouteIntent = (target,background=false) => {
    if (!window.history || config.feed === 'preview' && !background || !routeOwnerCurrent()
      || window.getSelection()?.toString()) return;
    const article = target.closest('article[data-testid="tweet"]');
    const root = article && nestedRoot(target,article);
    if (!root || !root.isConnected || target.closest('a[href]')) return;
    const users = [...root.querySelectorAll('[data-testid="User-Name"]')];
    if (users.length > 32) return;
    const ownUsers = users.filter(node => node.closest('article') === article && !nestedRoot(node,root));
    if (ownUsers.length !== 1) return;
    const authors=quoteAuthors(ownUsers[0],root,article);
    if (authors.size!==1) return;
    for (let node=root,depth=0;node && depth<64;node=node.parentElement,depth++) {
      const css=getComputedStyle(node);
      if (node.hidden || node.hasAttribute('inert') || node.getAttribute('aria-hidden')==='true'
        || css.display==='none' || css.visibility==='hidden' || css.visibility==='collapse' || css.opacity==='0') return;
      if (depth===63 && node.parentElement) return;
    }
    const intent={root,article,user:ownUsers[0],author:[...authors][0],background,path:location.pathname,epoch:visibilityEpoch,until:performance.now()+1000};
    routeIntent=intent;if(background)signal('focus');signal('routeIntent',background?'armBackground':'arm');
    setTimeout(()=>{if(routeIntent===intent)cancelRouteIntent();},1000);
  };
  if (window.history) {
    for (const name of ['pushState','replaceState']) {
      const original=window.history[name];
      if (typeof original !== 'function') continue;
      try {
        window.history[name]=function(...args) {
          const intent=routeIntent;
          if (this===window.history && intent && args.length>=3 && args[2]!=null) {
            const url=typeof args[2]==='string' ? local(args[2]) : null;
            // Same-document state bookkeeping is not an activation. The next
            // real destination consumes the intent even when it is rejected.
            if (!url || url.pathname!==location.pathname || url.search || url.hash) {
              const current=visible && intent.epoch===visibilityEpoch && intent.path===location.pathname
                && intent.root.isConnected && performance.now()<=intent.until
                && routeOwnerCurrent() && intent.root.contains(intent.user)
                && quoteAuthors(intent.user,intent.root,intent.article).size===1
                && quoteAuthors(intent.user,intent.root,intent.article).has(intent.author);
              const path=url && !url.search && !url.hash && navigationPath(url.href);
              cancelRouteIntent();
              if (current && path && !/\/(?:photo|video)\//.test(path)) {
                recoverSourceRoute(intent);
                signal(intent.background?'previewBackground':'preview',path);
                return;
              }
            }
          }
          historyRevision++;
          cancelRouteRecovery();
          return Reflect.apply(original,this,args);
        };
      } catch {} // Keep ordinary source navigation if its history API is sealed.
    }
  }
  // Diagnostics only: mirror the failed scope lookup without returning source
  // identifiers or changing native navigation. Release documents emit nothing.
  const routingProbe = (target, reason, scoped) => {
    if (config.diagnostics !== true || routingProbes >= 64 || target.closest('img,[data-testid="like"],[data-testid="unlike"],[data-testid="bookmark"],[data-testid="removeBookmark"],[data-testid="reply"],[data-testid="retweet"],[data-testid="unretweet"]')) return;
    const article = target.closest('article');
    if (!article || window.getSelection()?.toString()) return;
    const root = nestedRoot(target, article) || article;
    const own = node => node.closest('article') === article && !nestedRoot(node, root);
    const users = [...root.querySelectorAll('[data-testid="User-Name"]')].slice(0,32);
    const ownUsers = users.filter(own), user = ownUsers[0];
    const anchors = [...root.querySelectorAll('a[href]')].slice(0,128);
    const times = [...root.querySelectorAll('time')].slice(0,32);
    const timePaths = user ? [...user.querySelectorAll('time')].slice(0,32).filter(own)
      .map(node => postPath(node.closest('a')?.getAttribute('href'))).filter(Boolean) : [];
    const authors = user ? quoteAuthors(user, root, article) : new Set();
    const mediaPaths = anchors.filter(anchor => own(anchor) && anchor.querySelector('img,[data-testid="tweetPhoto"]'))
      .map(anchor => postPath(anchor.getAttribute('href'))).filter(path => path && /\/photo\/[1-4]$/.test(path))
      .map(path => path.replace(/\/photo\/[1-4]$/, ''));
    const uniqueMedia = new Set(mediaPaths);
    const role = root === article ? 'article' : root.matches('[data-testid="quoteTweet"],[data-testid="quotedTweet"]') ? 'quoteMarker' : root.matches('[role="link"]') ? 'link' : 'button';
    routingProbes++;
    signal('routeProbe', JSON.stringify({reason,role,hasTweetArticle:article.matches('[data-testid="tweet"]'),nested:root!==article,
      userNames:users.length,ownUserNames:ownUsers.length,anchors:anchors.length,times:times.length,timeLinks:timePaths.length,mediaLinks:mediaPaths.length,
      uniqueMedia:uniqueMedia.size===1,authorMatch:authors.size===1&&uniqueMedia.size===1&&authors.has([...uniqueMedia][0].split('/')[1].toLowerCase()),
      targetAnchor:!!target.closest('a[href]'),scopedResolved:!!scoped}));
  };
  const removeMenu = () => { const had=!!menu; menu?.remove(); menu = null; return had; };
  const dismissMenus = (cancelRouting=false) => {
    if (cancelRouting === true) { inputRevision++; scrollMediaIntent = null; cancelRouteIntent(); cancelRouteRecovery(); }
    const local=removeMenu(), actions=window.__atsumiXActions?.dismissMenus?.()===true;
    return local||actions;
  };
  const pause = () => document.querySelectorAll('video,audio').forEach(media => media.pause());
  const scrollScope = element => element?.closest('[role="dialog"],[aria-modal="true"]') || element?.closest('[data-testid="primaryColumn"]');
  const editing = () => document.activeElement instanceof Element && !!document.activeElement.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"])');
  const scrollMediaPresented = media => {
    const box = media.getBoundingClientRect();
    let left = Math.max(0,box.left), right = Math.min(innerWidth,box.right), top = Math.max(0,box.top), bottom = Math.min(innerHeight,box.bottom);
    if (!(box.width > 0 && box.height > 0 && right > left && bottom > top) || !recoveryPresented(media)) return false;
    let node = media.parentElement, depth = 0;
    for (;node && depth < 64;node = node.parentElement,depth++) {
      const css = getComputedStyle(node), clip = node.getBoundingClientRect();
      if (/^(hidden|clip|scroll|auto)$/.test(css.overflowX || css.overflow)) { left = Math.max(left,clip.left); right = Math.min(right,clip.right); }
      if (/^(hidden|clip|scroll|auto)$/.test(css.overflowY || css.overflow)) { top = Math.max(top,clip.top); bottom = Math.min(bottom,clip.bottom); }
      if (!(right > left && bottom > top)) return false;
    }
    return !node;
  };
  const consumeScrollMediaIntent = media => {
    const intent = scrollMediaIntent;
    scrollMediaIntent = null;
    return !!intent && visible && !document.hidden && !editing() && routeOwnerCurrent()
      && intent.epoch === visibilityEpoch && intent.input === inputRevision && intent.history === historyRevision
      && intent.href === location.href && performance.now() <= intent.until
      && intent.root.isConnected && intent.root.ownerDocument === document && media.isConnected
      && scrollScope(media) === intent.root && scrollMediaPresented(media);
  };
  let lastWheelClaim = -Infinity;
  document.addEventListener('wheel', event => {
    if (event.isTrusted) scrollMediaIntent = null;
    if (!event.isTrusted || !visible || document.hidden || event.ctrlKey || event.metaKey || event.altKey
      || !Number.isFinite(event.deltaX) || !Number.isFinite(event.deltaY) || !event.deltaX && !event.deltaY) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.isConnected || target.ownerDocument !== document
      || !target.closest('[data-testid="primaryColumn"],[role="dialog"],[aria-modal="true"]')
      || target.closest('[hidden],[aria-hidden="true"],[inert],input,textarea,[contenteditable="true"],.atsumi-x-download-menu,.atsumi-x-link-menu,.atsumi-x-save-notice')
      || window.__atsumiXAds?.blocked(target) || !routeOwnerCurrent() || editing() || !recoveryPresented(target)) return;
    const now = performance.now();
    // Source autoplay may fire before the asynchronous native wheel claim is
    // granted. Permit only its first visible current-pane play, not a new play()
    // call or an ongoing autoplay grant. Native registration/muting still wins.
    scrollMediaIntent = {root:scrollScope(target),href:location.href,epoch:visibilityEpoch,
      input:inputRevision,history:historyRevision,until:now+350};
    if (now - lastWheelClaim < 150) return;
    lastWheelClaim = now;
    signal('wheel');
  }, {capture:true,passive:true});
  const mediaRoot = element => element?.closest('video,audio,[data-testid="videoPlayer"]');
  const paneBackControl = target => {
    const button = target?.closest('button[data-testid="app-bar-back"],[role="button"][data-testid="app-bar-back"]');
    if (!button || !button.closest('[data-testid="primaryColumn"]')
      || button.closest('article,[role="dialog"],[aria-modal="true"],[data-testid="videoPlayer"]')
      || document.fullscreenElement || /\/(?:photo|video)\/[1-4]$/.test(postPath(location.pathname) || '')) return null;
    const scoped = config.feed === 'preview' && !!navigationPath(location.pathname)
      || ['likes','bookmarks'].includes(config.feed) && collectionKind(location.pathname) === config.feed;
    return scoped ? button : null;
  };
  const rememberMediaIntent = event => {
    if (!event.isTrusted || !visible || event.ctrlKey || event.metaKey || event.altKey || !(event.target instanceof Element)) return;
    if (event.target.closest('input,textarea,[contenteditable="true"]')) return;
    if (event.type === 'keydown' && ![' ','Enter','k','K'].includes(event.key)) return;
    if (['pointerdown','click'].includes(event.type) && event.button !== 0) return;
    const root = mediaRoot(event.target);
    if (root) {
      const now = performance.now();
      // The click following a pointerdown is the same gesture, not a second
      // manual token if the media already emitted play during pointerdown.
      if (event.type === 'click' && pointerRoot === root && now - pointerAt <= 1500) return;
      if (event.type === 'pointerdown') { pointerRoot = root; pointerAt = now; }
      mediaIntent = root; mediaIntentUntil = now + 1500;
    }
  };
  const focusedMediaFrame = () => {
    const frame = document.activeElement;
    if (!(frame instanceof Element) || frame.tagName !== 'IFRAME' || !frame.isConnected
      || !frame.closest('article[data-testid="tweet"],[data-testid="videoPlayer"]')
      || frame.closest('[hidden],[aria-hidden="true"],[inert]') || frame.hasAttribute('srcdoc')) return null;
    try {
      const url = new URL(frame.getAttribute('src'), location.origin);
      if (url.protocol !== 'https:' || url.port || url.username || url.password) return null;
      const youtube = ['www.youtube.com','youtube.com','www.youtube-nocookie.com','youtube-nocookie.com'].includes(url.hostname)
        && /^\/embed\/[A-Za-z0-9_-]{11}$/.test(url.pathname);
      const card = ['cards-frame.x.com','cards-frame.twitter.com'].includes(url.hostname) && /^\/cards(?:\/|$)/.test(url.pathname);
      if (!youtube && !card) return null;
      const box = frame.getBoundingClientRect();
      if (box.width < 80 || box.height < 40 || box.bottom <= 0 || box.right <= 0 || box.top >= innerHeight || box.left >= innerWidth) return null;
      let left = Math.max(0, box.left), right = Math.min(innerWidth, box.right);
      let top = Math.max(0, box.top), bottom = Math.min(innerHeight, box.bottom), node = frame, depth = 0;
      for (; node && depth < 64; node = node.parentElement, depth++) {
        const css = getComputedStyle(node);
        if (css.display === 'none' || css.visibility === 'hidden' || css.visibility === 'collapse' || css.opacity === '0') return null;
        if (node !== frame && /^(hidden|clip|scroll|auto)$/.test(css.overflowX || css.overflow)) {
          const clip = node.getBoundingClientRect(); left = Math.max(left, clip.left); right = Math.min(right, clip.right);
        }
        if (node !== frame && /^(hidden|clip|scroll|auto)$/.test(css.overflowY || css.overflow)) {
          const clip = node.getBoundingClientRect(); top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom);
        }
        if (right <= left || bottom <= top) return null;
      }
      if (node) return null; // An incomplete ancestry scan is not visibility proof.
      return frame;
    } catch { return null; }
  };
  // Cross-origin players do not bubble input/play events into the X document.
  // A real in-frame gesture still activates its ancestors and focuses the
  // iframe. Claim only that visible known player; never message its contents
  // or override its own mute/volume/autoplay settings. Losing focus to another
  // application is not an input claim.
  window.addEventListener('blur', event => {
    if (!event.isTrusted || !visible) return;
    const epoch = visibilityEpoch;
    setTimeout(() => {
      if (!visible || epoch !== visibilityEpoch || document.hidden || !document.hasFocus()
        || window.navigator?.userActivation?.isActive !== true || !focusedMediaFrame()) return;
      signal('focus');
      signal('play', 'manual');
    }, 0);
  });
  window.__atsumiXSurface = Object.freeze({
    visibility(value) { const next = value === true, resumed = next && !visible; if (next !== visible) { visibilityEpoch++; cancelRouteIntent(); cancelRouteRecovery(); dismissMenus(); } else if (!next) { cancelRouteRecovery(); dismissMenus(); } visible = next; window.__atsumiXInput?.visibility(visible); window.__atsumiXActions?.visibility(visible); if (!visible) { lastWheelClaim = -Infinity; scrollMediaIntent = null; window.__atsumiXAds?.clear(); stopPhotoProbe(); mediaIntent = null; pointerRoot = null; pause(); } else { if (resumed) window.__atsumiXAds?.update(verifiedOwner === config.owner); schedule(); } },
    refresh() { lastStatus = ''; schedule(); },
    pause,
    playback(value) { automaticPlayback = value === true; if (!automaticPlayback) pause(); },
    dismissMenu:dismissMenus,
    dismissMenus,
    archivePhotoOwner,
  });
  document.addEventListener('play', event => {
    if (!(event.target instanceof HTMLMediaElement) || event.target.ownerDocument !== document) return;
    if (!visible) { event.target.pause(); return; }
    // Revalidate source ad ownership at event time, before granting audio or
    // consuming a normal player's input intent. Never modify media URLs/mute.
    if (window.__atsumiXAds?.blocked(event.target)) { scrollMediaIntent = null; event.target.pause(); return; }
    if (!event.isTrusted) return;
    // Trusted media events also occur for autoplay. Only a recent, scoped,
    // trusted input is an explicit user claim; it is consumed once.
    const scrollManual = consumeScrollMediaIntent(event.target);
    const manual = scrollManual || !!mediaIntent && mediaIntent.contains(event.target) && performance.now() <= mediaIntentUntil;
    mediaIntent = null;
    if (!manual && !automaticPlayback) { event.target.pause(); return; }
    signal('play', manual ? 'manual' : 'auto'); // Never synthesize playback.
  }, true);
  document.addEventListener('click', event => {
    rememberMediaIntent(event);
    if (event.isTrusted) { cancelRouteIntent(); interruptRecovery(event); }
    if (openExternalLink(event)) return;
    if (!event.isTrusted || !visible || event.defaultPrevented || event.button !== 0 || event.shiftKey || event.altKey || event.metaKey) return;
    const background=event.ctrlKey===true;
    const target = event.target instanceof Element ? event.target : null;
    if (target && window.__atsumiXAds?.blocked(target)) return;
    // This app owns the pane boundary, not X's browser history. Block an
    // ordinary app-bar Back even during the observer's first render frame.
    // Native photo/modal/media Back controls remain entirely source-owned.
    if (paneBackControl(target)) { event.preventDefault(); event.stopImmediatePropagation(); return; }
    if (config.feed === 'preview' && !background) return;
    if (!target || target.closest('button,input,textarea,[contenteditable="true"],video,audio,[data-testid="videoPlayer"],[data-testid="tweetPhoto"],[role="dialog"],[aria-modal="true"]')) return;
    const anchor = target.closest('a[href]');
    const scoped = articlePath(target);
    // Some native quote cards are button-role containers. Permit only their
    // own exact author/time permalink, never a social control or parent post.
    if (target.closest('[role="button"]') && !scoped) { routingProbe(target,'missingScope',scoped); armRouteIntent(target,background); return; }
    const path = anchor ? navigationPath(anchor.getAttribute('href')) : scoped;
    if (!path) { routingProbe(target,'missingPath',scoped); armRouteIntent(target,background); return; }
    if (/\/(?:photo|video)\//.test(path)) return;
    if (window.getSelection()?.toString()) return;
    event.preventDefault(); event.stopImmediatePropagation(); if(background)signal('focus'); signal(background?'previewBackground':'preview', path);
  }, true);
  function openExternalLink(event) {
    if (!event.isTrusted || !visible || document.hidden || event.defaultPrevented || event.altKey
      || ![0,1].includes(event.button) || !routeOwnerCurrent()) return false;
    const target = event.target instanceof Element ? event.target : null;
    if (!target || !target.isConnected || target.ownerDocument !== document
      || target.closest('button,input,textarea,[contenteditable="true"],video,audio,[hidden],[aria-hidden="true"],[inert]')
      || window.__atsumiXAds?.blocked(target)) return false;
    const anchor = target.closest('a[href]');
    if (!anchor || anchor.hasAttribute('download')) return false;
    const url = externalLink(anchor.getAttribute('href'));
    if (!url) return false;
    if (event.button === 0 && window.getSelection()?.toString()) return false;
    event.preventDefault(); event.stopImmediatePropagation();
    cancelRouteIntent(); cancelRouteRecovery(); dismissMenus();
    signal('externalLink',url);
    return true;
  }
  document.addEventListener('auxclick', event => {
    if (event.button === 1) openExternalLink(event);
  }, true);
  document.addEventListener('contextmenu', event => {
    if (!event.isTrusted || !visible) return;
    cancelRouteIntent();
    interruptRecovery(event);
    const target = event.target instanceof Element ? event.target : null;
    if (target && window.__atsumiXAds?.blocked(target)) return;
    if (!target || target.closest('input,textarea,[contenteditable="true"]')) return;
    const anchor = target.closest('a[href]');
    const external = anchor ? externalLink(anchor.getAttribute('href')) : null;
    const path = anchor ? navigationPath(anchor.getAttribute('href')) : target.closest('article[data-testid="tweet"]') ? articlePath(target) : navigationPath(location.href);
    if (!path && !external) return;
    if (external && (document.hidden || !routeOwnerCurrent() || !target.isConnected || target.ownerDocument !== document
      || target.closest('[hidden],[aria-hidden="true"],[inert]'))) return;
    event.preventDefault(); event.stopImmediatePropagation(); dismissMenus();
    if (!document.querySelector('style[data-atsumi-x="link-menu"]')) {
      const style=document.createElement('style');style.dataset.atsumiX='link-menu';
      style.textContent='.atsumi-x-link-menu button:hover,.atsumi-x-link-menu button:focus-visible{background:#16181c!important}.atsumi-x-link-menu button:focus-visible{outline:2px solid #1d9bf0;outline-offset:-2px;border-radius:8px}';
      (document.head||document.documentElement).append(style);
    }
    menu = document.createElement('div'); menu.className='atsumi-x-link-menu'; menu.setAttribute('role','menu');menu.setAttribute('aria-label','링크 메뉴');
    menu.style.cssText = 'position:fixed;z-index:2147483647;width:232px;max-width:calc(100vw - 16px);max-height:calc(100vh - 16px);overflow:auto;box-sizing:border-box;background:#000;color:#e7e9ea;border:1px solid #2f3336;border-radius:12px;padding:4px;box-shadow:0 0 12px #ffffff30;font:14px system-ui';
    const button = document.createElement('button'); button.type='button';button.setAttribute('role','menuitem');button.textContent='외부 브라우저에서 열기';
    button.style.cssText='display:block;box-sizing:border-box;width:100%;padding:12px;border:0;border-radius:8px;background:transparent;color:inherit;font:inherit;text-align:left;cursor:pointer';
    const ownedMenu=menu;
    button.addEventListener('click', e=>{
      if(!e.isTrusted||!visible||menu!==ownedMenu||!button.isConnected)return;
      e.preventDefault();e.stopImmediatePropagation();removeMenu();
      if(!target.isConnected||window.__atsumiXAds?.blocked(target))return;
      const currentAnchor=target.closest('a[href]');
      if(external){
        if(!document.hidden && routeOwnerCurrent() && target.ownerDocument===document && !target.closest('[hidden],[aria-hidden="true"],[inert]')
          && currentAnchor===anchor && externalLink(currentAnchor?.getAttribute('href'))===external)signal('externalLink',external);
        return;
      }
      const currentPath=currentAnchor?navigationPath(currentAnchor.getAttribute('href')):
        target.closest('article[data-testid="tweet"]')?articlePath(target):navigationPath(location.href);
      if(currentPath===path)signal('external',path);
    });
    menu.append(button); document.body.append(menu);
    const width=menu.offsetWidth||232;
    menu.style.left=`${Math.max(8,Math.min(event.clientX-width+12,innerWidth-width-8))}px`;
    menu.style.top=`${Math.max(8,Math.min(event.clientY,innerHeight-menu.offsetHeight-8))}px`;
    button.focus();
  }, true);
  document.addEventListener('pointerdown', e=>{rememberMediaIntent(e);if(!e.isTrusted)return;interruptRecovery(e);cancelRouteIntent();if(menu&&!menu.contains(e.target))removeMenu();},true);
  document.addEventListener('keydown', e=>{
    rememberMediaIntent(e);
    interruptRecovery(e);
    if (!e.isTrusted || e.defaultPrevented || e.isComposing || e.ctrlKey || e.metaKey || e.altKey || e.target instanceof Element && e.target.closest('input,textarea,[contenteditable="true"]')) return;
    if(e.key==='Escape')cancelRouteIntent();
    if(e.key==='Escape'&&menu){e.preventDefault();e.stopImmediatePropagation();removeMenu();}
    // X owns Escape for its dialogs/photos. Only close the outer preview when
    // there is no official dialog, exact full-page photo, or native fullscreen.
    else if(!window.__atsumiXInput&&e.key==='Escape'&&config.feed==='preview'&&!/\/photo\/[1-4]$/.test(postPath(location.pathname)||'')&&!document.querySelector('[role="dialog"],[aria-modal="true"]')&&!document.fullscreenElement){e.preventDefault();signal('close');}
  },true);
  // Hidden documents must still complete their initial identity/render proof.
  // Visibility controls playback/interactions, not the readiness observer.
  function schedule() { if(!pending)pending=setTimeout(()=>{pending=0;update();},180); }
  function stopPhotoProbe(resetRoute = true) {
    photoProbeEpoch++;if(resetRoute)photoProbeRoute='';
    for(const timer of photoProbeTimers)clearTimeout(timer);
    photoProbeTimers=[];
  }
  function schedulePhotoProbe(state) {
    if(config.diagnostics!==true)return;
    const route=location.pathname;
    if(!visible||document.hidden||!/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(route)){
      if(photoProbeRoute)stopPhotoProbe();
      return;
    }
    if(photoProbeRoute&&photoProbeRoute!==route)stopPhotoProbe();
    if(state!=='ready'){
      // Losing readiness does not grant another four samples on this visit.
      if(photoProbeRoute)stopPhotoProbe(false);
      return;
    }
    if(photoProbeRoute===route)return;
    stopPhotoProbe();photoProbeRoute=route;
    const epoch=photoProbeEpoch;
    // At most four read-only samples per visible route visit. Do not force a
    // readiness update: the native status handler repaints the whole WebView.
    photoProbeTimers=[0,300,1000,3000].map((delay,index)=>setTimeout(()=>{
      if(epoch!==photoProbeEpoch||!visible||document.hidden||location.pathname!==route)return;
      signal('photoProbe',JSON.stringify({...photoDiagnosticShape(true),sample:index+1}));
    },delay));
  }
  function watchAccountPresentation(account, proven) {
    const next=proven&&account?.isConnected?account:null;
    if(next===watchedAccount)return;
    accountObserver?.disconnect();accountObserver=null;
    if(watchedAccount&&accountLoaded)watchedAccount.removeEventListener('load',accountLoaded,true);
    watchedAccount=next;accountLoaded=null;
    if(!next)return;
    // Only the exact currently proved switcher is observed, not the feed or
    // app-injected controls. A late src/srcset/currentSrc or text hydration
    // should not need a route change. The existing 180ms scheduler coalesces
    // bursts; there is no retry loop, extra navigation or remote lookup.
    const changed=()=>{if(watchedAccount===next&&next.isConnected)schedule();};
    accountObserver=new MutationObserver(changed);
    accountObserver.observe(next,{subtree:true,attributes:true,
      attributeFilter:['src','srcset','sizes','dir'],characterData:true});
    accountLoaded=event=>{
      if(event.target instanceof Element&&event.target.tagName==='IMG'&&next.contains(event.target))changed();
    };
    next.addEventListener('load',accountLoaded,true);
  }
  function accountAvatarCandidates(image) {
    if(!image)return [];
    // Read only URLs already attached to this exact proved account image.
    // Native validation separately requires the same avatar resource and host.
    const candidates=new Map();
    const add=(url,width=0)=>{
      if(typeof url!=='string'||!url||url.length>512||candidates.size>=8&&!candidates.has(url))return;
      if(!Number.isInteger(width)||width<0||width>8192)return;
      candidates.set(url,Math.max(candidates.get(url)||0,width));
    };
    const current=image.currentSrc||image.src||'';
    add(current,Math.min(image.naturalWidth||0,image.naturalHeight||0));
    add(image.src);
    const srcset=image.getAttribute('srcset')||'';
    if(srcset.length<=4096){
      for(const part of srcset.split(',').slice(0,8)){
        const match=/^\s*(https:\/\/[^\s,]+)\s+(\d+(?:\.\d+)?)\s*([wx])\s*$/.exec(part);
        if(!match)continue;
        const size=Number(match[2]);
        // Density-only declarations remain observed URLs, but do not invent a
        // pixel width from the hidden switcher's layout or device scale.
        if(match[3]==='w'&&Number.isInteger(size))add(match[1],size);
        else if(match[3]==='x'&&size>0&&size<=8)add(match[1]);
      }
    }
    return [...candidates].map(([url,width])=>({url,width}));
  }
  function trimHomeComposer(primary) {
    for (const marked of document.querySelectorAll('[data-atsumi-x-composer]')) marked.removeAttribute('data-atsumi-x-composer');
    if (config.feed === 'preview' || location.pathname !== '/home') return;
    // Hide only an identifiable global home composer. Replies and dialogs are
    // native X content, not redundant application chrome.
    for (const editor of [...primary.querySelectorAll('[data-testid="tweetTextarea_0"]')].slice(0,16)) {
      if (editor.closest('article,[role="dialog"],[aria-modal="true"]')) continue;
      let composer = null;
      for (let node = editor.parentElement; node && node !== primary; node = node.parentElement) {
        // X also gives its photo/GIF tools navigation and tablist roles. Only
        // button-bearing descendants without editors, links, tabs, panels or
        // document content may be tools inside this identified composer.
        const unknownTools = [...node.querySelectorAll('nav,[role="navigation"],[role="tablist"]')].some(tools =>
          !tools.querySelector('button,[role="button"]') || tools.querySelector('[data-testid="tweetTextarea_0"],[role="tab"],a[href],[role="link"],[role="tabpanel"],article,[role="dialog"],[aria-modal="true"],[role="region"],h1,h2,[role="heading"]'));
        if (node.matches('article,nav,[role="navigation"],[role="tablist"],[role="tab"],[role="dialog"],[aria-modal="true"],[role="region"]')
            || node.querySelector('article,[role="tab"],[role="dialog"],[aria-modal="true"],[role="region"],h1,h2,[role="heading"]') || unknownTools) break;
        if (node.querySelector('[data-testid="tweetButtonInline"],[data-testid="tweetButton"],[role="toolbar"]')) composer = node;
      }
      composer?.setAttribute('data-atsumi-x-composer','true');
    }
  }
  function nativePhotoReady(allowFullPage = false) {
    if (!/^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(location.pathname)) return false;
    const imageReady = img => {
      if (img.closest('[hidden],[aria-hidden="true"],[inert]')) return false;
      try {
        const url = new URL(img.currentSrc || img.getAttribute('src'));
        return url.protocol === 'https:' && url.hostname === 'pbs.twimg.com' && !url.port && !url.username && !url.password && /^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(url.pathname);
      } catch { return false; }
    };
    const dialogs = [...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')].slice(0,8);
    if (dialogs.length) return dialogs.some(dialog => !dialog.closest('[hidden],[aria-hidden="true"],[inert]')
      && !dialog.querySelector(photoSecurity) && [...dialog.querySelectorAll('img')].slice(0,32).some(imageReady));
    // Observed X also renders /photo/N as a full-page image without a modal or
    // primary column. This branch relies on this document's earlier owner proof,
    // never a route or image as an account identity, and cannot cover a gate.
    return allowFullPage && !document.querySelector(photoSecurity) && [...document.querySelectorAll('img')].slice(0,128).some(imageReady);
  }
  function archivePhotoOwner() {
    // Supplement the native current-account/URL/generation checks with this
    // document's already observed owner. A photo author is not an account proof.
    if (!visible || !verifiedOwner || verifiedOwner !== config.owner
      || !/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(location.pathname)
      || document.querySelector(photoSecurity)) return '';
    // Recheck every present profile now, before the asynchronous observer can
    // run. Missing responsive navigation is allowed; malformed or conflicting
    // current evidence is not. This read does not mutate the cached proof.
    const profiles = [...document.querySelectorAll('[data-testid="AppTabBar_Profile_Link"]')];
    if (profiles.length > 8 || profiles.some(node => {
      const url = local(node.getAttribute('href'));
      const owner = /^\/([A-Za-z0-9_]{1,15})$/.exec(url?.pathname || '')?.[1]?.toLowerCase();
      return owner !== config.owner;
    })) return '';
    return nativePhotoReady(true) ? verifiedOwner : '';
  }
  // Debug shape only. Do not use these broad observations as readiness or
  // authentication proof, and never include source strings in the payload.
  function photoDiagnosticShape(includeAction = false) {
    const path = location.pathname, exact = postPath(path);
    const route = path === '/login' || path.startsWith('/i/flow/') ? 'auth'
      : exact && /\/photo\/[1-4]$/.test(exact) ? 'photo'
      : /\/photo\//.test(path) ? 'photoOther'
      : exact && /\/video\/[1-4]$/.test(exact) ? 'video'
      : exact ? 'post' : path === '/home' ? 'home'
      : collectionKind(path) ? 'collection' : navigationPath(path) ? 'profile' : 'other';
    const dialogs = [...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')].slice(0,8);
    const images = [...new Set(dialogs.flatMap(dialog => [...dialog.querySelectorAll('img')].slice(0,32)))].slice(0,32);
    const imageShape = img => {
      try {
        const url = new URL(img.currentSrc || img.getAttribute('src'));
        const host = url.protocol === 'https:' && url.hostname === 'pbs.twimg.com' && !url.port && !url.username && !url.password;
        return {host,media:host && /^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(url.pathname)};
      } catch { return {host:false,media:false}; }
    };
    return {route,segments:Math.min(path.split('/').filter(Boolean).length,16),dialogs:dialogs.length,dialogImages:images.length,
      photoCandidates:Math.min(document.querySelectorAll('[data-testid="tweetPhoto"]').length,32),
      dialogImageHost:images.some(img=>imageShape(img).host),dialogImageMedia:images.some(img=>imageShape(img).media),
      pageImageMedia:[...document.querySelectorAll('img')].slice(0,128).some(img=>imageShape(img).media),
      hiddenDialog:dialogs.some(dialog=>!!dialog.closest('[hidden],[aria-hidden="true"],[inert]')),
      securityInput:!!document.querySelector(photoSecurity),
      ...(includeAction?{actionShape:window.__atsumiXActions?.photoDiagnostic?.()||null}:{})};
  }
  function accountDiagnosticShape(account) {
    // Debug-only structure of the proven account switcher. Never include its
    // text, labels, attributes, resource URLs or another post author's avatar.
    const nodes=[];
    if(account){
      const walker=document.createTreeWalker(account,1);
      for(let node=walker.currentNode;node&&nodes.length<128;node=walker.nextNode())nodes.push(node);
    }
    const images=nodes.filter(node=>node.tagName==='IMG').slice(0,32);
    const hidden=node=>{
      for(let current=node,depth=0;current instanceof Element&&depth<32;current=current.parentElement,depth++){
        const css=getComputedStyle(current);
        if(current.hidden||current.hasAttribute('inert')||current.getAttribute('aria-hidden')==='true'
          ||css.display==='none'||css.visibility==='hidden'||css.visibility==='collapse')return true;
      }
      return false;
    };
    return {nodes:nodes.length,images:images.length,
      imageSources:images.filter(img=>!!(img.currentSrc||img.getAttribute('src'))).length,
      loadedImages:images.filter(img=>img.naturalWidth>0).length,
      backgrounds:Math.min(nodes.filter(node=>/url\(/i.test(getComputedStyle(node).backgroundImage||'')).length,32),
      textLength:Math.min((account?.textContent||'').trim().length,512),
      autoTextNodes:Math.min(nodes.filter(node=>node.getAttribute('dir')==='auto').length,32),
      ariaLabel:!!account?.hasAttribute('aria-label'),
      avatarContainers:Math.min(nodes.filter(node=>(node.getAttribute('data-testid')||'').startsWith('UserAvatar-Container-')).length,8),
      hidden:!!account&&hidden(account),hasBox:!!account&&account.getClientRects().length>0,
      narrow:innerWidth<700};
  }
  function collectionKind(path) {
    if (['/i/bookmarks','/i/history/bookmarks','/i/history'].includes(path)) return 'bookmarks';
    if (path === '/i/history/likes' || path === `/${config.owner}/likes`) return 'likes';
    return null;
  }
  function trimCollectionTabs(primary) {
    for (const marked of document.querySelectorAll('[data-atsumi-x-collection-tabs]')) marked.removeAttribute('data-atsumi-x-collection-tabs');
    if (!['likes','bookmarks'].includes(config.feed) || collectionKind(location.pathname) !== config.feed) return;
    for (const list of [...primary.querySelectorAll('[role="tablist"]')].slice(0,16)) {
      if (list.closest('article,[role="dialog"],[aria-modal="true"]') || list.querySelector('input,textarea,article,[role="dialog"],[aria-modal="true"],h1,h2,[role="heading"]')) continue;
      const tabs = [...list.querySelectorAll('[role="tab"]')].filter(tab => tab.closest('[role="tablist"]') === list);
      if (tabs.length !== 2) continue;
      const kinds = tabs.map(tab => {
        const anchors = tab.matches('a[href]') ? [tab] : [...tab.querySelectorAll('a[href]')];
        if (anchors.length) return anchors.length === 1 ? collectionKind(local(anchors[0].getAttribute('href'))?.pathname) : null;
        const label = tab.textContent.trim();
        return /^(bookmarks|북마크|ブックマーク)$/i.test(label) ? 'bookmarks' : /^(likes|마음에\s*들어요|いいね)$/i.test(label) ? 'likes' : null;
      });
      if (kinds.includes('likes') && kinds.includes('bookmarks')) list.setAttribute('data-atsumi-x-collection-tabs','true');
    }
  }
  function trimPaneBack(primary) {
    for (const marked of document.querySelectorAll('[data-atsumi-x-pane-back]')) marked.removeAttribute('data-atsumi-x-pane-back');
    for (const button of [...primary.querySelectorAll('[data-testid="app-bar-back"]')].slice(0,16)) {
      paneBackControl(button)?.setAttribute('data-atsumi-x-pane-back','true');
    }
  }
  function update() {
    const profile = local(document.querySelector('a[data-testid="AppTabBar_Profile_Link"]')?.getAttribute('href'));
    const observedOwner = /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(profile?.pathname || '')?.[1]?.toLowerCase();
    const account = document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
    const accountImage=account?.querySelector('img');
    const nameParts=account?[...account.querySelectorAll('[dir="auto"]')].filter(n=>!n.querySelector('[dir="auto"]')).map(n=>(n.textContent||'').trim()).filter(t=>t&&t.length<=120&&!t.startsWith('@')):[];
    const avatarCandidates=accountAvatarCandidates(observedOwner===config.owner?accountImage:null);
    const accountPresentation=observedOwner===config.owner&&account?{handle:observedOwner,name:nameParts[0]||'',avatar:accountImage?.currentSrc||accountImage?.src||null,...(avatarCandidates.length?{avatarCandidates}:{} )}:null;
    const login = location.pathname==='/login'||location.pathname.startsWith('/i/flow/')||!observedOwner&&!!document.querySelector('a[href="/i/flow/login"],a[href="/login"]');
    const ownerConflict=!!observedOwner&&!!config.owner&&observedOwner!==config.owner;
    watchAccountPresentation(account,!!account&&observedOwner===config.owner&&!login);
    if(login||ownerConflict){verifiedOwner=null;scrollMediaIntent=null;cancelRouteIntent();cancelRouteRecovery();dismissMenus();}
    else if(observedOwner&&account)verifiedOwner=observedOwner;
    // Responsive X can remove desktop navigation after the pane is resized.
    // Keep only this document's earlier proof, never infer identity from posts.
    const owner=observedOwner&&account?observedOwner:verifiedOwner;
    let state = login ? 'loginRequired' : 'loading';
    if(!login&&ownerConflict)state='ownerChanged';
    const primary = document.querySelector('[data-testid="primaryColumn"]');
    const photo = nativePhotoReady(!primary && owner === config.owner && verifiedOwner === config.owner && !ownerConflict && !login);
    if(owner&&state!=='ownerChanged'&&state!=='loginRequired'&&(primary||photo)) {
      state='ready';
      if(!applied) {
        const style=document.createElement('style');style.dataset.atsumiX='surface';
        style.textContent='header[role="banner"],[data-testid="sidebarColumn"],[data-testid="BottomBar"],[data-testid="SideNav_NewTweet_Button"],[data-testid="FloatingActionButton_Tweet"],[data-atsumi-x-composer="true"],[data-atsumi-x-collection-tabs="true"],[data-atsumi-x-pane-back="true"]{display:none!important}html,body,#react-root{min-width:0!important;background-color:#000!important}main[role="main"]{width:100%!important;max-width:none!important;min-width:0!important;flex:1!important;background-color:#000!important}main[role="main"]>div{width:100%!important;max-width:none!important}[data-testid="primaryColumn"]{width:100%!important;max-width:600px!important;min-width:0!important;margin:0 auto!important;border-left:0!important;border-right:0!important;background-color:#000!important}';
        document.head.append(style);applied=true;
      }
      if(primary){trimHomeComposer(primary);trimCollectionTabs(primary);trimPaneBack(primary);}
      window.__atsumiXAds?.update(visible && owner === config.owner);
      window.__atsumiXActions?.update();
      if(primary&&(config.feed==='foryou'||config.feed==='following')&&location.pathname==='/home') {
        const tabs=[...primary.querySelectorAll('[role="tablist"] [role="tab"]')];
        const wanted=tabs.find(tab=>(config.feed==='following'?/^(following|팔로잉|フォロー中)$/i:/^(for you|추천|おすすめ)$/i).test(tab.textContent.trim()));
        // Select the requested official home tab once per document. Afterwards
        // the user's own X tab selection is respected.
        if(!config.homeSelected&&wanted){config.homeSelected=true;if(wanted.getAttribute('aria-selected')!=='true')wanted.click();}
      }
      // Only ordinary per-post content warnings already accessible to this
      // account. Never a login/age check, account preference, or paid gate.
      for(const warning of visible&&primary?[...primary.querySelectorAll('[data-testid="tweet"] [data-testid="contentWarning"], [data-testid="tweet"] [data-testid="sensitiveMediaWarning"]')].slice(0,20):[]) {
        if(window.__atsumiXAds?.blocked(warning))continue;
        const text=warning.textContent||'';
        if(!/sensitive|민감|センシティブ/i.test(text)||/age|verify|18|연령|나이|인증|subscribe|구독/i.test(text))continue;
        const button=[...warning.querySelectorAll('button,[role="button"]')].find(b=>/^(show|view|표시|보기|표시하기|表示)$/i.test(b.textContent.trim()));
        if(button&&!revealed.has(button)){revealed.add(button);button.click();}
      }
    }
    if(state!=='ready')window.__atsumiXAds?.clear();
    schedulePhotoProbe(state);
    const statusValue={state,owner:owner||null,path:location.pathname,accountPresentation,proof:{profile:!!observedOwner,account:!!account,primary:!!primary,photo,login:!!login,...(config.diagnostics===true?{photoShape:photoDiagnosticShape(),accountShape:accountDiagnosticShape(account),adShape:window.__atsumiXAds?.diagnostic()||null}:{})}};
    let status=JSON.stringify(statusValue);
    // Optional portrait candidates must not overflow the existing authenticated
    // 4096-character bridge envelope and discard ordinary readiness updates.
    const prefix=`https://x.com/__atsumi_surface__/${config.nonce}?kind=status&value=`;
    while(accountPresentation?.avatarCandidates?.length&&prefix.length+encodeURIComponent(status).length>4096){
      const candidates=accountPresentation.avatarCandidates;
      const least=candidates.reduce((index,candidate,i)=>candidate.width<candidates[index].width?i:index,0);
      candidates.splice(least,1);
      if(!candidates.length)delete accountPresentation.avatarCandidates;
      status=JSON.stringify(statusValue);
    }
    if(status!==lastStatus){lastStatus=status;signal('status',status);}
  }
  const observer=new MutationObserver(schedule);
  const observe=()=>{if(document.documentElement)observer.observe(document.documentElement,{childList:true,subtree:true});schedule();};
  if(document.documentElement)observe();
  window.addEventListener('popstate',event=>{if(event!==recoveryEvent){historyRevision++;scrollMediaIntent=null;cancelRouteRecovery();}cancelRouteIntent();dismissMenus();if(photoProbeRoute!==location.pathname)stopPhotoProbe();schedule();});document.addEventListener('visibilitychange',()=>{if(document.hidden){scrollMediaIntent=null;cancelRouteIntent();cancelRouteRecovery();dismissMenus();stopPhotoProbe();pause();}else schedule();});
  document.addEventListener('DOMContentLoaded',observe,{once:true});schedule();
})()
