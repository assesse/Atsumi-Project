// Read only rendered official feed/collection articles. No cookies, storage, application
// state, tokens, private API endpoints or account-changing actions are accessed.
(() => {
  const request = __ATSUMI_X_REQUEST__;
  const result = (state, extra = {}) => ({ nonce: request.nonce, state, ...extra });
  if (window !== window.top || location.protocol !== 'https:' || !['x.com', 'www.x.com'].includes(location.hostname)) return result('unavailable');
  if (!['foryou','following','communities','likes','bookmarks'].includes(request.feed)) return result('unavailable');
  if (location.pathname === '/login' || location.pathname.startsWith('/i/flow/')) return result('signedOut');
  const profile = document.querySelector('a[data-testid="AppTabBar_Profile_Link"]');
  const account = document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
  const localPath = raw => { try { const url = new URL(raw, location.origin || 'https://' + location.hostname); return url.protocol === 'https:' && url.hostname === location.hostname && !url.port && !url.username && !url.password && !url.search && !url.hash ? url.pathname : null; } catch { return null; } };
  const handle = /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(localPath(profile?.getAttribute('href')) || '')?.[1]?.toLowerCase();
  if (!handle || !account) {
    if (document.querySelector('a[href="/login"],a[href="/i/flow/login"]')) return result('signedOut');
    return result('pending');
  }
  if (request.owner && request.owner !== handle) return result('ownerChanged');
  const collection = ['communities','likes','bookmarks'].includes(request.feed);
  const paths = request.feed === 'likes' ? [`/${handle}/likes`] : request.feed === 'bookmarks' ? ['/i/bookmarks'] : request.feed === 'communities' ? ['/i/communities',`/${handle}/communities`] : ['/home'];
  if (!paths.includes(location.pathname)) {
    if (request.target) return result('pending'); // A native-validated navigation is still settling.
    if (location.pathname !== '/home' || request.advance) return result('unavailable');
    // Prefer a rendered navigation link when available. Native code validates
    // this closed route set again; neither arbitrary hrefs nor posts can route it.
    const observed = [...document.querySelectorAll('nav a[href]')].map(link => link.getAttribute('href')).find(path => paths.includes(path));
    return result('navigate', {owner:handle, path:observed || paths[0]});
  }
  // Collections require the owner proved on home before native navigation.
  if (collection && (!request.owner || request.target !== location.pathname)) return result('unavailable');
  const primary = document.querySelector('[data-testid="primaryColumn"]');
  if (!primary) return result('pending');
  const tabs = [...primary.querySelectorAll('[role="tablist"] [role="tab"]')];
  const headings = [...primary.querySelectorAll('h1,h2,[role="heading"]')].map(node => node.textContent.trim());
  const expected = request.feed === 'following' ? /^(following|팔로잉)$/i : request.feed === 'likes' ? /^(likes|마음에 들어요|마음에 들어한 글|마음함|좋아요)$/i : request.feed === 'communities' ? /^(your communities|my communities|joined|joined communities|내 커뮤니티|가입한 커뮤니티|가입된 커뮤니티|가입 중)$/i : /^(for you|추천)$/i;
  if (request.feed === 'bookmarks') {
    if (!headings.some(text => /^(bookmarks|all bookmarks|북마크|모든 북마크)$/i.test(text))) return result('unsupported', {owner:handle});
  } else {
    const target = tabs.find(tab => request.feed === 'likes' && localPath(tab.getAttribute('href') || tab.querySelector('a[href]')?.getAttribute('href')) === `/${handle}/likes` || expected.test(tab.textContent.trim().replace(/\s+/g,' ')));
    // A plain Communities/For you heading is NOT evidence of membership.
    const joinedHeading = request.feed === 'communities' && !tabs.length && headings.some(text => /^(your communities|my communities|내 커뮤니티|가입한 커뮤니티|가입된 커뮤니티)$/i.test(text));
    // Official help describes the no-tab Communities root as the member
    // timeline. Require its rendered sidebar entry and community-tagged posts;
    // never accept a For you/Discover tab or a /suggested destination.
    const memberRoot = request.feed === 'communities' && !tabs.length && location.pathname === '/i/communities'
      && headings.some(text => /^(communities|커뮤니티)$/i.test(text))
      && [...document.querySelectorAll('nav a[href]')].some(link => localPath(link.getAttribute('href')) === '/i/communities')
      && [...primary.querySelectorAll('article[data-testid="tweet"]')].some(article => [...article.querySelectorAll('a[href]')].some(link => /^\/i\/communities\/[1-9]\d+$/.test(localPath(link.getAttribute('href')) || '')));
    if (!target && !joinedHeading && !memberRoot) return result('unsupported', {owner:handle, reason:'collection-context'});
    if (target && target.getAttribute('aria-selected') !== 'true') {
      if (request.advance) return result('unsupported', {owner:handle});
      target.click(); // Read-only official feed selection, never account actions.
      return result('pending', {owner:handle});
    }
  }
  const articles = [...primary.querySelectorAll('article[data-testid="tweet"]')].filter(article => !article.parentElement?.closest('article[data-testid="tweet"]')).slice(0, 60);
  const context = {owner:handle, path:location.pathname};
  if (request.advance) {
    articles.at(-1)?.scrollIntoView({ block: 'start', behavior: 'instant' });
    window.scrollBy({ top: 600, behavior: 'instant' });
    return result('advanced', context);
  }
  const posts = [], seen = new Set();
  const insideQuote = (node, article) => {
    for (let parent = node.parentElement; parent && parent !== article; parent = parent.parentElement) {
      // X gives real <a> anchors role="link" too; only a non-anchor linked
      // card is a quote boundary. Rejecting every role=link loses all posts.
      if (parent.getAttribute('role') === 'link' && parent.tagName !== 'A') return true;
    }
    return false;
  };
  const safeMedia = raw => {
    try {
      const url = new URL(raw);
      if (url.protocol !== 'https:' || url.hostname !== 'pbs.twimg.com' || url.username || url.password || url.port || !/^\/media\/[A-Za-z0-9_-]+(?:\.(?:jpg|jpeg|png|webp))?$/.test(url.pathname)) return null;
      if (url.searchParams.has('format') && !['jpg','jpeg','png','webp'].includes(url.searchParams.get('format'))) return null;
      url.searchParams.set('name','medium'); return url.href;
    } catch { return null; }
  };
  const safeAvatar = raw => {
    try { const url = new URL(raw); return url.protocol === 'https:' && url.hostname === 'pbs.twimg.com' && !url.port && !url.username && !url.password && !url.search && !url.hash && /^\/profile_images\/[1-9]\d{0,24}\/[A-Za-z0-9_-]+\.(?:jpg|jpeg|png|webp)$/.test(url.pathname) ? url.href : null; } catch { return null; }
  };
  for (const article of articles) {
    // A quoted/reposted article must not supply the enclosing post's identity.
    const time = [...article.querySelectorAll('time')].find(node => node.closest('article') === article && node.closest('[data-testid="User-Name"]') && !insideQuote(node, article));
    const anchor = time?.closest('a');
    let match;
    try { const url = new URL(anchor?.href); if (!['x.com','www.x.com'].includes(url.hostname) || url.protocol !== 'https:') continue; match = /^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(url.pathname); } catch { continue; }
    if (!match || seen.has(match[2])) continue;
    const user = time.closest('[data-testid="User-Name"]');
    const text = [...article.querySelectorAll('[data-testid="tweetText"]')].find(node => node.closest('article') === article && !insideQuote(node, article))?.textContent || '';
    const media = [...article.querySelectorAll('[data-testid="tweetPhoto"] img')].filter(node => node.closest('article') === article && !insideQuote(node, article)).map(img => safeMedia(img.currentSrc || img.src)).filter(Boolean).slice(0,4);
    const name = [...user.querySelectorAll('a')].find(link => !link.href.includes('/status/'))?.textContent?.trim() || match[1];
    const avatar = [...article.querySelectorAll('[data-testid^="UserAvatar-Container"] img')].filter(node => !insideQuote(node, article) && node.closest('article') === article).map(img => safeAvatar(img.currentSrc || img.src)).find(Boolean) || null;
    seen.add(match[2]);
    posts.push({ id:match[2], handle:match[1], name:name.slice(0,120), text:text.slice(0,6000), truncated:text.length>6000, time:(time.dateTime || '').slice(0,40), media, avatar, video:!!article.querySelector('[data-testid="videoPlayer"]') });
  }
  if (!posts.length && !articles.length) {
    const empty = primary.querySelector('[data-testid="emptyState"]')?.textContent?.trim() || '';
    const evidence = request.feed === 'likes' ? /hasn.t liked any|haven.t liked|no likes yet|아직.*마음에|마음에.*게시물이 없/i : request.feed === 'bookmarks' ? /save posts for later|haven.t added any.*bookmark|no bookmarks|아직.*북마크|나중에.*게시물을 저장/i : request.feed === 'communities' ? /haven.t joined|no communities yet|아직.*커뮤니티.*가입|가입한 커뮤니티가 없/i : /welcome to x|타임라인에 오신 것을 환영/i;
    if (evidence.test(empty)) return result('empty', {...context, posts:[]});
  }
  return result(posts.length ? 'ready' : 'pending', { ...context, posts });
})()
