// Read only. This runs only in a native-validated, current, visible X pane.
// No private state, authentication material, source HTML, or network requests.
(() => {
  const DIAGNOSTICS = false;
  const LIMIT = 80, TEXT = 24000;
  const profileOwners = [...document.querySelectorAll('a[data-testid="AppTabBar_Profile_Link"][href]')].map(a => { try { const u = new URL(a.href, location.href); return u.origin === 'https://x.com' && /^\/[A-Za-z0-9_]{1,15}$/.test(u.pathname) ? u.pathname.slice(1).toLowerCase() : ''; } catch { return ''; } });
  const handles = new Set(profileOwners.filter(Boolean));
  let owner = document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]') && handles.size === 1 && profileOwners.every(Boolean) ? [...handles][0] : '';
  const visible = node => { for (let n = node; n instanceof Element; n = n.parentElement) { const s = getComputedStyle(n); if (n.hidden || n.hasAttribute('hidden') || n.hasAttribute('inert') || n.getAttribute('aria-hidden') === 'true' || s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse' || s.opacity === '0') return false; } return node instanceof Element; };
  const text = (node, max = TEXT) => (node?.innerText || node?.textContent || '').trim().slice(0, max);
  // Display names may consist entirely of X's rendered emoji images. Admit
  // only visible, bounded Unicode emoji alt, not arbitrary image descriptions.
  const emojiAlt = value => typeof value === 'string' && value.length <= 32 &&
    /^(?:\p{Regional_Indicator}{2}|[0-9#*]\uFE0F?\u20E3|\p{Extended_Pictographic}(?:\uFE0F|\p{Emoji_Modifier})?(?:\u200D\p{Extended_Pictographic}(?:\uFE0F|\p{Emoji_Modifier})?)*)+$/u.test(value) ? value : '';
  const displayedText = (node, max = 120, skipMetadata = false) => {
    let value = '', visited = 0, overflow = false;
    const walk = (current, depth) => {
      if (++visited > 256 || depth > 24) { overflow = true; return; }
      if (value.length > max) return;
      if (current.nodeType === 3) { value += current.textContent || ''; return; }
      if (!(current instanceof Element) || !visible(current) || current.matches('script,style,svg')) return;
      if (skipMetadata) {
        const label = displayedText(current, 120);
        const previous = current.previousElementSibling, next = current.nextElementSibling;
        const timeSeparator = /^[·•]$/.test(label) && previous &&
          /^@[A-Za-z0-9_]{1,15}$/.test(displayedText(previous)) &&
          next && (next.matches('time') || next.querySelector('time'));
        if (current.matches('time,[data-testid="icon-verified"]') ||
          /^@[A-Za-z0-9_]{1,15}$/.test(label) || timeSeparator) return;
      }
      if (current.tagName === 'IMG') { value += emojiAlt(current.getAttribute('alt')); return; }
      if (current.tagName === 'BR') { value += '\n'; return; }
      for (const child of current.childNodes) walk(child, depth + 1);
    };
    if (node && visible(node)) walk(node, 0);
    return overflow ? '' : value.trim().slice(0, max);
  };
  const contentText = (node, max = TEXT + 1) => {
    if (!node || !visible(node)) return { value:'', limited:false };
    const descendants = [...node.querySelectorAll('*')], changed = new Set();
    if (descendants.length > 4096) return { value:'', limited:true };
    // Preserve the source's own innerText whitespace for untouched branches.
    // Only paths containing images or non-displayed/unsafe text are rebuilt.
    for (const child of [node, ...descendants]) {
      if (!child.matches('img,script,style,svg') && visible(child)) continue;
      let current=child, depth=0;
      while (current && node.contains(current) && depth++<=64) { changed.add(current); current=current.parentElement; }
      if (depth>64) return { value:'', limited:true };
    }
    let limited=false, visited=0;
    const walk = (current, depth) => {
      if (++visited>8192 || depth>64) { limited=true; return ''; }
      if (current.nodeType===3) return current.textContent || '';
      if (!(current instanceof Element) || !visible(current) || current.matches('script,style,svg')) return '';
      if (current.tagName==='IMG') return emojiAlt(current.getAttribute('alt'));
      if (current.tagName==='BR') return '\n';
      if (!changed.has(current)) return current.innerText || current.textContent || '';
      let value='';
      for (const child of current.childNodes) {
        const part=walk(child,depth+1);
        if (!part) continue;
        const block=child instanceof Element && /^(block|flex|grid|list-item|table-row)$/.test(getComputedStyle(child).display);
        if (block && value && !value.endsWith('\n') && !part.startsWith('\n')) value+='\n';
        value+=part;
        if (block && !value.endsWith('\n')) value+='\n';
        if (value.length>max) { limited=true; return value.slice(0,max); }
      }
      return value;
    };
    const value=walk(node,0).trim();
    return { value:value.slice(0,max), limited:limited || value.length>max };
  };
  const postUrl = value => {
    try { const u = new URL(value, location.href); return u.origin === 'https://x.com' && /^\/[A-Za-z0-9_]{1,15}\/status\/[0-9]{1,24}$/.test(u.pathname) ? u.origin + u.pathname : null; } catch { return null; }
  };
  const mediaUrl = value => {
    try { const u = new URL(value); if (u.protocol !== 'https:' || !['pbs.twimg.com', 'video.twimg.com'].includes(u.hostname) || u.username || u.password || u.port || u.hash || u.href.length > 2048 || u.pathname.includes('%')) return null;
      if (u.hostname === 'pbs.twimg.com') { if (!/^\/(media|profile_images|ext_tw_video_thumb|amplify_video_thumb|tweet_video_thumb)\/[A-Za-z0-9_./-]+$/.test(u.pathname) || [...u.searchParams.keys()].some(k => !['format', 'name'].includes(k))) return null; }
      else if (![...u.searchParams.keys()].every(k => ['tag', 'container', 'v', 'variant_version'].includes(k))) return null;
      return u.href;
    } catch { return null; }
  };
  const paintedBackground = (image, url, loadedQuote = false) => {
    const style=getComputedStyle(image), parent=image.parentElement;
    // X's image component can use a transparent sizing img beside its actual
    // paint. Only the img's own opacity may be bypassed, never hidden structure.
    if(style.opacity!=='0'||image.hidden||image.hasAttribute('inert')||image.getAttribute('aria-hidden')==='true'
      ||style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse'
      ||!parent||parent.tagName!=='DIV'||parent.children.length>8||!visible(parent))return false;
    const rect=image.getBoundingClientRect();
    // Whole-post capture includes already loaded offscreen DOM. A quote's
    // exact painted avatar follows that rule; current-photo capture still
    // requires viewport presence. Neither path permits zero/invalid geometry.
    const viewport=r=>['left','right','top','bottom','width','height'].every(key=>Number.isFinite(r[key]))&&
      r.width>0&&r.height>0&&(loadedQuote||r.right>0&&r.bottom>0&&r.left<window.innerWidth&&r.top<window.innerHeight);
    const matches=node=>{const r=node.getBoundingClientRect();return viewport(r)&&['left','right','top','bottom'].every(key=>Math.abs(r[key]-rect[key])<=1);};
    if(!viewport(rect)||!matches(parent))return false;
    const paints=[parent,...parent.children].filter(node=>node!==image&&getComputedStyle(node).backgroundImage&&getComputedStyle(node).backgroundImage!=='none');
    if(paints.length!==1)return false;
    const paint=paints[0];
    if(paint!==parent&&(paint.tagName!=='DIV'||paint.childElementCount!==0||(paint.textContent||'').trim()))return false;
    if(!visible(paint)||!matches(paint))return false;
    // One literal URL only: no layers, image-set, gradients, escaping or guesses.
    const background=/^url\((?:"([^"\\]+)"|'([^'\\]+)'|([^"'()\\\s]+))\)$/.exec(getComputedStyle(paint).backgroundImage);
    return !!background&&mediaUrl(background[1]||background[2]||background[3])===url;
  };
  // Diagnostic-only mirror of the paint proof. Closed stages contain no source
  // values and never decide resource admission or alter the displayed document.
  const paintFailure = (image,url,loadedQuote = false) => {
    const style=getComputedStyle(image), parent=image.parentElement;
    if(style.opacity!=='0'||image.hidden||image.hasAttribute('inert')||image.getAttribute('aria-hidden')==='true'||
      style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse')return 'paintOwnStyle';
    if(!parent||parent.tagName!=='DIV'||parent.children.length>8||!visible(parent))return 'paintParent';
    const rect=image.getBoundingClientRect();
    const viewport=r=>['left','right','top','bottom','width','height'].every(key=>Number.isFinite(r[key]))&&
      r.width>0&&r.height>0&&(loadedQuote||r.right>0&&r.bottom>0&&r.left<window.innerWidth&&r.top<window.innerHeight);
    const matches=node=>{const r=node.getBoundingClientRect();return viewport(r)&&['left','right','top','bottom'].every(key=>Math.abs(r[key]-rect[key])<=1);};
    if(!viewport(rect)||!matches(parent))return 'paintRect';
    const paints=[parent,...parent.children].filter(node=>node!==image&&getComputedStyle(node).backgroundImage&&getComputedStyle(node).backgroundImage!=='none');
    if(paints.length!==1)return 'paintLayers';
    const paint=paints[0];
    if(paint!==parent&&(paint.tagName!=='DIV'||paint.childElementCount!==0||(paint.textContent||'').trim())||
      !visible(paint)||!matches(paint))return 'paintShape';
    const background=/^url\((?:"([^"\\]+)"|'([^'\\]+)'|([^"'()\\\s]+))\)$/.exec(getComputedStyle(paint).backgroundImage);
    return background&&mediaUrl(background[1]||background[2]||background[3])===url ? null : 'paintMatch';
  };
  const scope = value => {
    try { const u = new URL(value); const m = u.pathname.match(/^\/(ext_tw_video(?:_thumb)?|amplify_video(?:_thumb)?)\/([0-9]+)\//); if (m) return m[1].replace('_thumb', '') + '/' + m[2]; const t = u.pathname.match(/^\/(tweet_video(?:_thumb)?)\/([^/.]+)/); return t ? 'tweet_video/' + t[2] : null; } catch { return null; }
  };
  const observed = performance.getEntriesByType('resource').map(e => mediaUrl(e.name)).filter(Boolean).slice(-500);
  const records = [], seen = new Set(), recordNodes = new Map();
  let quoteDiagnostic=null;
  const detailUrl = postUrl(location.origin + location.pathname);
  const precedes = (first, second) => !!(first.compareDocumentPosition(second) & 4);
  const profileHandle = value => {
    try { const u = new URL(value, location.href); return u.origin === 'https://x.com' && /^\/[A-Za-z0-9_]{1,15}$/.test(u.pathname) ? u.pathname.slice(1) : ''; } catch { return ''; }
  };
  const authorPresentation = (user, url) => {
    const candidates = [...user?.querySelectorAll('a[href],span') || []];
    const observedNodes = candidates.length <= 256 ? candidates : [];
    const observedHandles = observedNodes.filter(visible).flatMap(node => {
      const profile = node.matches('a[href]') ? profileHandle(node.href) : '';
      const label = /^@([A-Za-z0-9_]{1,15})$/.exec(displayedText(node));
      return [profile, label?.[1]].filter(Boolean);
    });
    const unique = new Map(observedHandles.map(handle => [handle.toLowerCase(), handle]));
    const handle = url ? new URL(url).pathname.split('/')[1] : unique.size === 1 ? [...unique.values()][0] : '';
    const links = observedNodes.filter(link => link.matches('a[href]') && visible(link) && profileHandle(link.href).toLowerCase() === handle.toLowerCase());
    const name = links.map(link => displayedText(link)).find(value => value && !/^@[A-Za-z0-9_]{1,15}$/.test(value)) ||
      (!url ? displayedText(user, 120, true) : '');
    // A missing badge/name is not evidence of an unverified account or a name.
    return { handle, displayName:name, verified:[...user?.querySelectorAll('[data-testid="icon-verified"]') || []].some(visible) ? true : null };
  };
  const displayedCount = node => {
    if (!node) return '';
    const label = node.getAttribute('aria-label')?.trim() || '';
    // Accessible labels avoid concatenating old/new animated digit nodes.
    const value = label.match(/^(\d[\d,.]*(?:\s*[kKmMbB천만억])?)(?=\s|$)/)?.[1];
    return (value || text(node, 32)).slice(0, 32);
  };
  const photoPath = location.origin === 'https://x.com' && /^\/([A-Za-z0-9_]{1,15})\/status\/([0-9]{1,24})\/photo\/[1-4]$/.exec(location.pathname);
  const security = 'input[type="password"],input[autocomplete="one-time-code"],[data-testid="ocfEnterTextTextInput"],[data-testid="LoginForm_Login_Button"],a[href^="/i/flow/"],a[href="/login"]';
  const inViewport = node => { const r=node.getBoundingClientRect();return r.width>0&&r.height>0&&r.right>0&&r.bottom>0&&r.left<window.innerWidth&&r.top<window.innerHeight; };
  const dialogs = photoPath ? [...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')] : [];
  const visibleDialogs = dialogs.length<=8 ? dialogs.filter(node=>visible(node)&&inViewport(node)) : [];
  // A nested wrapper and its inner dialog are one surface. Two unrelated
  // visible layers cannot be ordered safely from source DOM alone.
  const topDialogs = visibleDialogs.filter(node=>!visibleDialogs.some(other=>other!==node&&node.contains(other)));
  const photoDialog = topDialogs.length===1 ? topDialogs[0] : null;
  const modalPhoto = !!photoPath&&dialogs.length>0;
  for (const article of document.querySelectorAll('article[data-testid="tweet"]')) {
    if (!visible(article) || article.parentElement?.closest('article[data-testid="tweet"]') || records.length >= LIMIT) continue;
    if (modalPhoto&&(!photoDialog||!photoDialog.contains(article))) continue;
    // The time permalink belongs to the outer post; quoted cards are separate.
    const inArticle = selector => [...article.querySelectorAll(selector)].filter(n => n.closest('article[data-testid="tweet"]') === article);
    const quoteRoots = inArticle('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]').filter(n => n.matches('[data-testid="quoteTweet"],[data-testid="quotedTweet"]') || (n.querySelector('[data-testid="User-Name"]') && n.querySelector('[data-testid="tweetText"],[data-testid="tweetPhoto"]')));
    const own = selector => inArticle(selector).filter(n => !quoteRoots.some(q => q.contains(n)));
    const anchor = own('a[href] time').map(n => n.closest('a')).find(a => postUrl(a?.href));
    const url = postUrl(anchor?.href); if (!url) continue;
    const id = url.split('/').pop(); if (seen.has(id)) continue; seen.add(id);
    const author = own('[data-testid="User-Name"]')[0];
    const media = [], urls = new Set();
    const add = (kind, value, alt = '') => { const u = mediaUrl(value); if (!u) return null; const old = media.find(m => m.url === u); if (old) return old; if (media.length >= 16) return null; urls.add(u); const entry = { kind, url: u, alt: alt.slice(0, 2000) }; media.push(entry); return entry; };
    for (const photo of inArticle('[data-testid="tweetPhoto"]')) { if (photo.querySelector('video,[data-testid="videoPlayer"],[data-testid="videoComponent"]')) continue; const img = photo.querySelector('img'); if (!add('image', img?.currentSrc || img?.src, img?.alt || '') && media.length < 16) media.push({ kind: 'unavailable_image', url: '', alt: 'No permitted image resource was displayed at capture time.' }); }
    for (const video of inArticle('video')) {
      const source = mediaUrl(video.currentSrc || video.src), posterScope = scope(video.poster);
      const candidates = source ? [source] : observed.filter(u => posterScope && scope(u) === posterScope && /\.(mp4|m3u8)(?:\?|$)/.test(u));
      // Prefer the observed top-level playlist, never construct a new variant.
      const playlists = candidates.filter(u => /\.m3u8(?:\?|$)/.test(u)).sort((a, b) => new URL(a).pathname.length - new URL(b).pathname.length);
      const choice = playlists[0] || candidates[0];
      if (choice) { const entry = add('video', choice); if (entry) entry.poster = mediaUrl(video.poster); } else if (media.length < 16) media.push({ kind: 'unavailable_video', url: '', poster: mediaUrl(video.poster), alt: 'No permitted video resource was observed at capture time.' });
      if (video.poster) add('poster', video.poster);
    }
    const avatar = own('[data-testid="Tweet-User-Avatar"] img,[data-testid^="UserAvatar-Container"] img')[0];
    const ownAvatar = avatar && add('avatar', avatar.currentSrc || avatar.src, avatar.alt || '');
    const bodyResults = own('[data-testid="tweetText"]').map(n => contentText(n));
    const bodies = bodyResults.map(result => result.value);
    const body = bodies.join('\n').slice(0, TEXT);
    const quoteResults = quoteRoots.filter(n => !quoteRoots.some(other => other !== n && other.contains(n))).map(n => contentText(n));
    const quoteTexts = quoteResults.map(result => result.value);
    const quotes = quoteTexts.map(t => t.slice(0, 12000)).slice(0, 2);
    const counts = own('[role="group"]')[0]?.getAttribute('aria-label') || '';
    const control = value => own(`[data-testid="${value}"]`)[0];
    const presentation = { ...authorPresentation(author, url), counts:{
      replies:displayedCount(control('reply')),
      reposts:displayedCount(control('unretweet') || control('retweet')),
      likes:displayedCount(control('unlike') || control('like')),
      bookmarks:displayedCount(control('removeBookmark') || control('bookmark')),
      views:displayedCount(own('a[href$="/analytics"]')[0]),
    }, relation:'unknown', order:records.length };
    const ownTime = anchor?.querySelector('time');
    const timestampLabel = ownTime && visible(ownTime) ? text(ownTime, 80) : '';
    if (timestampLabel) presentation.timestampLabel = timestampLabel;
    if (ownAvatar) presentation.avatarUrl = ownAvatar.url;
    const ownResources = new Set(own('[data-testid="tweetPhoto"] img,video').flatMap(node =>
      [mediaUrl(node.currentSrc || node.src), node.tagName === 'VIDEO' ? mediaUrl(node.poster) : null]).filter(Boolean));
    // Resource storage is deduplicated, but visual occurrences are not. The
    // same file may be shown both in this post and in its quoted card.
    presentation.mediaUrls = media.filter(item => ['image','video','poster'].includes(item.kind) &&
      (ownResources.has(item.url) || item.poster && ownResources.has(item.poster))).map(item => item.url);
    const quote = quoteRoots.filter(node => !quoteRoots.some(other => other !== node && other.contains(node)))[0];
    const inspectQuote=DIAGNOSTICS && (detailUrl ? url===detailUrl : records.length===0);
    if(inspectQuote)quoteDiagnostic={
      detailRoot:!!detailUrl,quoteFound:!!quote,quoteVisible:!!quote&&visible(quote),
      authorUnambiguous:false,conflictingTargets:false,selected:false,admitted:false,mediaFull:false,
      quoteRoots:Math.min(quoteRoots.length,65),nestedRoots:quote?Math.min(quoteRoots.filter(other=>other!==quote&&quote.contains(other)).length,65):0,
      aliases:0,authorNodes:0,contentNodes:0,links:0,images:0,known:0,knownInScope:0,
      profiles:0,notProfile:0,bodyOrMedia:0,nested:0,visible:0,painted:0,
      hidden:0,ariaHidden:0,ancestorHidden:0,opacityZero:0,
      paintOwnStyle:0,paintParent:0,paintRect:0,paintLayers:0,paintShape:0,paintMatch:0,candidates:0,
      selectedParentDiv:false,selectedAncestorComplete:false,selectedAncestorLimit:false,
      selectedParentChildren:0,selectedAncestors:0,selectedAncestorAriaHidden:0,selectedAncestorHidden:0,
      selectedAncestorInert:0,selectedAncestorDisplayNone:0,selectedAncestorVisibilityHidden:0,selectedAncestorOpacityZero:0
    };
    if (quote) {
      const quoteAnchor = [...quote.querySelectorAll('a[href] time')].map(node => node.closest('a')).find(link => postUrl(link?.href));
      const observedPermalinks = new Set([quote, ...quote.querySelectorAll('a[href]')].filter(node =>
        !node.closest('[data-testid="tweetText"]') && (node === quote || !node.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"]') || node.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"]') === quote))
        .map(node => postUrl(node.getAttribute('href'))).filter(Boolean));
      const quoteUrl = postUrl(quoteAnchor?.href) || (observedPermalinks.size === 1 ? [...observedPermalinks][0] : null);
      if (visible(quote)) {
        const quoteTime = quoteAnchor?.querySelector('time') || quote.querySelector('time');
        const quoteView = {
          ...authorPresentation(quote.querySelector('[data-testid="User-Name"]'), quoteUrl),
          body:contentText(quote.querySelector('[data-testid="tweetText"]'), 12000).value,
          timestamp:quoteTime && visible(quoteTime) ? quoteTime.getAttribute('datetime') || '' : '' };
        // An observed card without a unique permalink is presentation only.
        // It grants no post identity, selection or conversation membership.
        if (quoteUrl) presentation.quote = { id:quoteUrl.split('/').pop(), url:quoteUrl, ...quoteView };
        else presentation.quotePresentation = quoteView;
        const target = presentation.quote || presentation.quotePresentation;
        const quoteTimeLabel = quoteTime && visible(quoteTime) ? text(quoteTime, 80) : '';
        if (quoteTimeLabel) target.timestampLabel = quoteTimeLabel;
        // X can expose both a quote wrapper and its inner role-link for one
        // card. Node identity, not matching text/handles, proves that overlap.
        // This only narrows avatar exclusion; it grants no canonical identity.
        const quoteAuthors=[...quote.querySelectorAll('[data-testid="User-Name"]')];
        const quoteContent=[...quote.querySelectorAll('[data-testid="tweetText"],[data-testid="tweetPhoto"],video')];
        const quoteLinks=[quote,...quote.querySelectorAll('[href]')];
        const quoteTargets=new Set(quoteLinks.filter(node=>!node.closest('[data-testid="tweetText"]')).map(node=>postUrl(node.getAttribute('href'))).filter(Boolean));
        const redundantWrappers=new Set();
        if(quoteRoots.length<=32 && quoteAuthors.length===1 && visible(quoteAuthors[0]) &&
          authorPresentation(quoteAuthors[0],null).handle && quoteContent.length>0 && quoteContent.length<=32 &&
          quoteLinks.length<=64 && quoteTargets.size<=1) {
          for(const candidate of quoteRoots) {
            if(candidate===quote || !quote.contains(candidate) || !visible(candidate))continue;
            const authors=[...candidate.querySelectorAll('[data-testid="User-Name"]')];
            const content=[...candidate.querySelectorAll('[data-testid="tweetText"],[data-testid="tweetPhoto"],video')];
            if(authors.length===1 && authors[0]===quoteAuthors[0] && content.length===quoteContent.length &&
              content.every((node,index)=>node===quoteContent[index]))redundantWrappers.add(candidate);
          }
        }
        const quoteImageScope = image => !image.closest('[data-testid="tweetText"],[data-testid="tweetPhoto"]') &&
          !quoteRoots.some(other=>other!==quote && quote.contains(other) && other.contains(image) && !redundantWrappers.has(other));
        const quoteImagePresented = image => {
          const url=mediaUrl(image.currentSrc || image.src);
          return url && new URL(url).hostname==='pbs.twimg.com' && new URL(url).pathname.startsWith('/profile_images/') &&
            (visible(image) || paintedBackground(image,url,true));
        };
        const knownAvatars=[...quote.querySelectorAll('[data-testid="Tweet-User-Avatar"] img,[data-testid^="UserAvatar-Container"] img')].filter(quoteImageScope);
        let quoteAvatar = knownAvatars.length===1 ? knownAvatars[0] : null;
        // Some native quote cards omit avatar test IDs. Only one visible,
        // observed profile image in this exact card is a safe fallback.
        if (!knownAvatars.length) {
          const images=[...quote.querySelectorAll('img')];
          const candidates=images.length<=64 ? images.filter(image => quoteImageScope(image) && quoteImagePresented(image)) : [];
          if (candidates.length===1) quoteAvatar=candidates[0];
        }
        if(inspectQuote)quoteDiagnostic.mediaFull=media.length>=16;
        const admitted = quoteAvatar && quoteImagePresented(quoteAvatar) && add('avatar', quoteAvatar.currentSrc || quoteAvatar.src, quoteAvatar.alt || '');
        if (admitted) target.avatarUrl = admitted.url;
        if(inspectQuote) {
          const shape=quoteDiagnostic, images=[...quote.querySelectorAll('img')];
          shape.aliases=Math.min(redundantWrappers.size,65);
          shape.authorNodes=Math.min(quoteAuthors.length,65);
          shape.authorUnambiguous=quoteAuthors.length===1&&!!authorPresentation(quoteAuthors[0],null).handle;
          shape.contentNodes=Math.min(quoteContent.length,65);shape.links=Math.min(quoteLinks.length,65);
          shape.conflictingTargets=quoteTargets.size>1;shape.images=Math.min(images.length,65);
          shape.known=Math.min(quote.querySelectorAll('[data-testid="Tweet-User-Avatar"] img,[data-testid^="UserAvatar-Container"] img').length,65);
          shape.knownInScope=Math.min(knownAvatars.length,65);shape.selected=!!quoteAvatar;shape.admitted=!!admitted;
          // Diagnostic only: distinguish accessibility exclusion from actual
          // hidden paint without changing either resource-admission gate.
          // Inspect only this selected image's ancestors through its own quote.
          if(quoteAvatar) {
            let ancestor=quoteAvatar.parentElement;
            shape.selectedParentDiv=ancestor?.tagName==='DIV';
            shape.selectedParentChildren=Math.min(ancestor?.childElementCount||0,65);
            while(ancestor instanceof Element && quote.contains(ancestor) && shape.selectedAncestors<32) {
              const style=getComputedStyle(ancestor);
              shape.selectedAncestors++;
              if(ancestor.getAttribute('aria-hidden')==='true')shape.selectedAncestorAriaHidden++;
              if(ancestor.hidden||ancestor.hasAttribute('hidden'))shape.selectedAncestorHidden++;
              if(ancestor.hasAttribute('inert'))shape.selectedAncestorInert++;
              if(style.display==='none')shape.selectedAncestorDisplayNone++;
              if(style.visibility==='hidden'||style.visibility==='collapse')shape.selectedAncestorVisibilityHidden++;
              if(style.opacity==='0')shape.selectedAncestorOpacityZero++;
              if(ancestor===quote){shape.selectedAncestorComplete=true;break;}
              ancestor=ancestor.parentElement;
            }
            shape.selectedAncestorLimit=!shape.selectedAncestorComplete &&
              shape.selectedAncestors===32 && ancestor instanceof Element && quote.contains(ancestor);
          }
          for(const image of images.slice(0,64)) {
            const resource=mediaUrl(image.currentSrc||image.src);
            if(!resource||new URL(resource).hostname!=='pbs.twimg.com'||!new URL(resource).pathname.startsWith('/profile_images/')) {shape.notProfile++;continue;}
            shape.profiles++;
            if(image.closest('[data-testid="tweetText"],[data-testid="tweetPhoto"]'))shape.bodyOrMedia++;
            if(quoteRoots.some(other=>other!==quote&&quote.contains(other)&&other.contains(image)&&!redundantWrappers.has(other)))shape.nested++;
            const style=getComputedStyle(image);
            if(image.hidden||image.hasAttribute('inert')||style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse')shape.hidden++;
            if(image.getAttribute('aria-hidden')==='true')shape.ariaHidden++;
            if(!visible(image.parentElement))shape.ancestorHidden++;
            if(style.opacity==='0')shape.opacityZero++;
            if(visible(image))shape.visible++;
            else {const failure=paintFailure(image,resource,true);if(failure)shape[failure]++;else shape.painted++;}
            if(quoteImageScope(image)&&quoteImagePresented(image))shape.candidates++;
          }
        }
        const displayedResources = new Set([...quote.querySelectorAll('[data-testid="tweetPhoto"] img,video')].flatMap(node =>
          [mediaUrl(node.currentSrc || node.src), node.tagName === 'VIDEO' ? mediaUrl(node.poster) : null]).filter(Boolean));
        const quoteMedia = media.filter(item => ['image','video','poster'].includes(item.kind) &&
          (displayedResources.has(item.url) || item.poster && displayedResources.has(item.poster))).map(item => item.url);
        if (quoteMedia.length) target.mediaUrls = quoteMedia;
        const ownMedia = own('[data-testid="tweetPhoto"],video').filter(node => visible(node) && !node.contains(quote));
        if (ownMedia.length && ownMedia.every(node => precedes(node, quote))) presentation.mediaBeforeQuote = true;
        else if (ownMedia.length && ownMedia.every(node => precedes(quote, node))) presentation.mediaBeforeQuote = false;
      }
    }
    records.push({ id, url, author: text(author, 1000), body, timestamp: anchor?.querySelector('time')?.getAttribute('datetime') || '', counts: counts.slice(0, 2000), context: text(own('[data-testid="socialContext"]')[0], 1000), quotes, media, presentation, truncated: own('[data-testid="tweet-text-show-more-link"]').length > 0 || bodyResults.some(result=>result.limited) || quoteResults.some(result=>result.limited) || bodies.join('\n').length > TEXT || quoteTexts.length > 2 || quoteTexts.some(t => t.length > 12000) || media.length >= 16 });
    recordNodes.set(id, { article, quoteRoots });
  }
  // Only a current canonical detail root establishes a conversation. A feed
  // neighbour, ancestor post or quote never becomes a reply because of order.
  const root = detailUrl && records.find(record => record.url === detailUrl);
  if (root) {
    const rootNode = recordNodes.get(root.id).article;
    const primary = rootNode.closest('[data-testid="primaryColumn"],main,[role="main"]');
    const conversation = node => {
      for (let n=node.parentElement;n;n=n.parentElement)
        if (/^(timeline: conversation|타임라인: 대화|タイムライン: 会話)$/i.test(n.getAttribute('aria-label') || '')) return n;
      return null;
    };
    const rootConversation = conversation(rootNode);
    const scope = rootConversation || primary;
    const headings = scope ? [...scope.querySelectorAll('h1,h2,h3,[role="heading"]')].filter(node =>
      !node.closest('article[data-testid="tweet"]') && visible(node) && precedes(rootNode, node)) : [];
    root.presentation.relation = 'root'; root.presentation.conversationId = root.id;
    const admittedPosts = new Set([root.url]);
    for (const record of records) {
      if (record === root) continue;
      const { article, quoteRoots } = recordNodes.get(record.id);
      // A new rendered section heading ends the proved conversation slice,
      // including unfamiliar locales/source revisions. Do not guess its role.
      if (!scope || !scope.contains(article) || !precedes(rootNode, article) || headings.some(heading => precedes(heading, article))) continue;
      const ownMetadata = node => node.closest('article[data-testid="tweet"]') === article &&
        !node.closest('[data-testid="tweetText"]') && !node.querySelector('[data-testid="tweetText"]') &&
        !quoteRoots.some(quote => quote.contains(node) || node.contains(quote));
      const markers = [...article.querySelectorAll('[data-testid="replyingTo"],[data-testid="replyingToText"],div')].filter(node =>
        ownMetadata(node) && (node.matches('[data-testid="replyingTo"],[data-testid="replyingToText"]') ||
          node.children.length < 4 && (/^(replying to(?:\s|$)|답글 대상|返信先)/i.test(text(node, 300)) || /님에게 보내는 답글$/.test(text(node, 300)))));
      // An author-only "Replying to @name" may refer to another thread by that
      // author. Without the explicit Conversation scope, require an observed
      // exact parent permalink, not just a matching name, handle or DOM order.
      const targetedReply = markers.some(marker => [...marker.querySelectorAll('a[href]')].some(link => admittedPosts.has(postUrl(link.href))));
      if (!(rootConversation && conversation(article) === rootConversation) && !targetedReply) continue;
      record.presentation.relation = 'reply'; record.presentation.conversationId = root.id;
      admittedPosts.add(record.url);
    }
  }
  function resolveCurrentCarouselPhoto(candidates, route, shown, trace=null) {
    // undefined preserves the legacy unambiguous non-carousel path. A detected
    // but conflicting/incomplete carousel returns null, including mid-transition.
    // This optional closed diagnostic reports the existing first rejection;
    // it never changes admission or exports source state, CSS or dimensions.
    const done=(proof,result=null)=>{if(trace)trace.carouselProof=proof;return result;};
    if (!Array.isArray(candidates) || !candidates.length) return done('emptyCandidates');
    if (candidates.length > 128) return done('candidateLimit');
    const match = /^\/[A-Za-z0-9_]{1,15}\/status\/[0-9]{1,25}\/photo\/([1-4])$/.exec(route);
    if (!match) return done('route');
    const index = Number(match[1]), listSelector = '[role="list"],ul,ol', itemSelector = '[role="listitem"],li';
    const isCarousel = node => node?.getAttribute('role') === 'carousel' ||
      /^(carousel|회전 슬라이드|カルーセル)$/i.test(node?.getAttribute('aria-roledescription') || '');
    const carouselOf = image => {
      let node = image, depth = 0;
      for (; node && depth < 32; node = node.parentElement, depth++) if (isCarousel(node)) return node;
      return node ? false : null;
    };
    const roots = candidates.map(candidate => carouselOf(candidate.image));
    if (roots.some(root => root === false)) return done('ancestorLimit');
    if (roots.every(root => root === null)) { if(trace)trace.carouselProof='absent'; return undefined; }
    const root = roots[0];
    if (!root || roots.some(other => other !== root)) return done('mixedRoots');
    if (!shown(root)) return done('hiddenRoot');
    if (carouselOf(root.parentElement) !== null) return done('nestedRoot');
    const lists = [...root.querySelectorAll(listSelector)];
    if (lists.length !== 1) return done('lists');
    const list = lists[0], items = [...list.querySelectorAll(itemSelector)];
    if (items.length < 1 || items.length > 4 || index > items.length) return done('items');
    if (items.some(item => item.closest(listSelector) !== list || item.parentElement?.closest(itemSelector))) return done('nestedItems');
    const parseCounter = value => {
      if (typeof value !== 'string' || value.length > 80) return null;
      const match = /^(?:slide\s+)?([1-4])\s*(?:\/|of)\s*([1-4])(?:\s*-\s*carousel)?$/i.exec(value.trim()) ||
        /^슬라이드\s*([1-4])\s*\/\s*([1-4])\s*-\s*회전 슬라이드$/.exec(value.trim());
      return match && Number(match[1]) <= Number(match[2]) ? [Number(match[1]), Number(match[2])] : null;
    };
    const counters = [];
    for (const node of [root, root.previousElementSibling]) {
      if (!node || !shown(node) || node.closest('article[data-testid="tweet"]')) continue;
      const label = parseCounter(node.getAttribute('aria-label'));
      if (label) counters.push(label);
      if (node.childElementCount <= 4 && node.querySelectorAll('*').length <= 8) {
        const text = (node.textContent || '').trim(), counter = text.length <= 80 ? parseCounter(text) : null;
        if (counter) counters.push(counter);
      }
    }
    if (!counters.length) return done('counterMissing');
    if (counters.some(([current, total]) => current !== index || total !== items.length)) return done('counterMismatch');
    const currentItem = items[index - 1];
    if (!shown(currentItem)) return done('currentHidden');
    for (let ordinal = 0; ordinal < items.length; ordinal++) {
      const item = items[ordinal];
      for (const [name, expected] of [['aria-posinset', ordinal + 1], ['aria-setsize', items.length]]) {
        if (item.hasAttribute(name) && item.getAttribute(name) !== String(expected)) return done(name==='aria-posinset'?'positionConflict':'sizeConflict');
      }
      for (const name of ['aria-current', 'aria-selected']) {
        if (!items.some(other => other.hasAttribute(name))) continue;
        const value = item.getAttribute(name), selected = value !== null && value !== '' && value !== 'false';
        if (name === 'aria-selected' && value !== null && !['true', 'false'].includes(value)) return done('selectedConflict');
        if (selected !== (item === currentItem)) return done(name==='aria-current'?'currentConflict':'selectedConflict');
      }
    }
    const exposed = image => {
      const original = image.getBoundingClientRect();
      if (![original.left, original.right, original.top, original.bottom, original.width, original.height].every(Number.isFinite) ||
          original.width <= 0 || original.height <= 0) return done('imageRect');
      let left = Math.max(original.left, 0), right = Math.min(original.right, window.innerWidth),
        top = Math.max(original.top, 0), bottom = Math.min(original.bottom, window.innerHeight),
        node = image, depth = 0, absoluteUnbound = false, fixedBoundary = false;
      const fixedContainingBlock = style =>
        ['transform','translate','rotate','scale','perspective','filter','backdropFilter'].some(name => style[name] && style[name] !== 'none')
        || style.contain && style.contain !== 'none' || style.willChange && style.willChange !== 'auto'
        || style.containerType && style.containerType !== 'normal'
        || style.contentVisibility && style.contentVisibility !== 'visible'
        || style.transformStyle && style.transformStyle !== 'flat';
      for (; node && depth < 32; node = node.parentElement, depth++) {
        const style = getComputedStyle(node);
        // A viewport-fixed viewer is not a fixed image escaping its slide.
        // Keep its own/inner clip proof, but never let an outer overflow box
        // falsely hide a viewport-fixed neighbour. Transformed/contained outer
        // blocks make that boundary uncertain and remain rejected.
        if (fixedBoundary) { if (fixedContainingBlock(style)) return done('fixedAncestor'); continue; }
        if (style.position === 'fixed' && node === image) return done('fixedImage');
        if (style.position === 'fixed' && fixedContainingBlock(style)) return done('fixedAncestor');
        if (node !== image && /^(relative|absolute|sticky|fixed)$/.test(style.position)) absoluteUnbound = false;
        if (node === image) { absoluteUnbound = style.position === 'absolute'; continue; }
        const rect = node.getBoundingClientRect();
        if (![rect.left, rect.right, rect.top, rect.bottom].every(Number.isFinite)) return done('ancestorRect');
        // Do not assume overflow clips an absolute descendant whose containing
        // block is outside it. Unknown transform-only containing blocks fail
        // closed; a positioned parent inside the clip is sufficient proof.
        const clips = /^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowX || style.overflow) ||
          /^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowY || style.overflow);
        if (clips && absoluteUnbound) return done('unboundAbsolute');
        if (/^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowX || style.overflow)) {
          left = Math.max(left, rect.left); right = Math.min(right, rect.right);
        }
        if (/^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowY || style.overflow)) {
          top = Math.max(top, rect.top); bottom = Math.min(bottom, rect.bottom);
        }
        if (style.position === 'absolute') absoluteUnbound = true;
        if (style.position === 'fixed') fixedBoundary = true;
      }
      return node ? done('clipDepth') : right > left && bottom > top;
    };
    let current = null;
    for (const candidate of candidates) {
      if (!candidate.image?.isConnected) return done('detached');
      const item = candidate.image.closest(itemSelector);
      if (!item || !items.includes(item) || candidate.image.closest(listSelector) !== list) return done('candidateScope');
      const presented = exposed(candidate.image);
      if (presented === null) return null;
      if (item === currentItem) {
        if (current) return done('currentDuplicate');
        if (!presented) return done('currentClipped');
        current = candidate;
      } else if (presented) return done('neighbourExposed');
    }
    return done(current?'ready':'currentMissing',current);
  }
  // X photos can remove the account chrome and every article. A
  // read-only proof from this document's presentation adapter is required; the
  // route or the post author's name is never used as an account identity.
  let modalAccepted = false;
  if (photoPath && !document.querySelector(security)) {
    let proven = '';
    try { const value = window.__atsumiXSurface?.archivePhotoOwner?.(); if (typeof value === 'string' && /^[A-Za-z0-9_]{1,15}$/.test(value)) proven = value.toLowerCase(); } catch {}
    if (proven && profileOwners.every(value => value === proven) && (!owner || owner === proven)) {
      if (!owner) owner = proven;
      const photoRoot=modalPhoto?photoDialog:
        !records.length&&!document.querySelector('article[data-testid="tweet"],[role="dialog"],[aria-modal="true"]')?document:null;
      if (photoRoot) {
        const candidates = new Map(), standalone = new Map();
        const imageNodes = photoRoot.querySelectorAll('img');
        // A partial scan cannot prove uniqueness of the displayed photo.
        for (const image of imageNodes.length <= 128 ? imageNodes : []) {
          const url = mediaUrl(image.currentSrc || image.src);
          if (!url) continue;
          const parsed = new URL(url);
          if (parsed.hostname !== 'pbs.twimg.com' || !/^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(parsed.pathname)) continue;
          if (!visible(image) && !paintedBackground(image,url)) continue;
          if (!inViewport(image)) continue;
          const entry={ kind: 'image', url, alt: (image.alt || '').slice(0, 2000) };
          candidates.set(url,{image,entry});
          if(!image.closest('article[data-testid="tweet"]'))standalone.set(url,{image,entry});
        }
        // The photo canvas can sit next to a sidebar with its own post media.
        // Prefer that independent visible canvas; never capture the background
        // feed or synthesize the rest of the post from the current photo.
        const current=modalPhoto&&standalone.size?standalone:candidates;
        const resolved=resolveCurrentCarouselPhoto([...current.values()],location.pathname,visible);
        const selected=resolved===undefined?(current.size===1?[...current.values()][0]:null):resolved;
        if (selected) {
          const url=`https://x.com/${photoPath[1]}/status/${photoPath[2]}`,media=[selected.entry];
          const full=modalPhoto?records.find(post=>post.url===url&&post.media.some(item=>item.kind==='image'&&item.url===media[0].url)):null;
          if(modalPhoto)records.length=0;
          records.push(full||{
            id: photoPath[2], url, author: '', body: '', timestamp: '', counts: '', context: '', quotes: [],
            media, truncated: false, photoOnly: true,
          });
          modalAccepted=modalPhoto;
        }
      }
    }
  }
  // Photo-modal capture is scoped to its admitted current image even when an
  // ordinary article happened to be visible underneath an unknown/gated layer.
  if(modalPhoto&&!modalAccepted)records.length=0;
  const result = { version: 1, sourceUrl: location.origin + location.pathname, owner, capturedAt: new Date().toISOString(), coverage: 'loaded_dom_only', records, limitReached: records.length >= LIMIT };
  if(DIAGNOSTICS&&quoteDiagnostic&&records.length)result.quoteDiagnostic=quoteDiagnostic;
  // Enabled only by the debug native caller. Counts describe this same capture
  // without changing any admission gate or retaining source strings.
  if (DIAGNOSTICS && !records.length) {
    const images = document.querySelectorAll('img'), candidates = new Set();
    const diagnostic = {
      photoRoute:!!photoPath, articles:Math.min(document.querySelectorAll('article[data-testid="tweet"]').length,81),
      dialogs:Math.min(document.querySelectorAll('[role="dialog"],[aria-modal="true"]').length,9),
      images:Math.min(images.length,129), imagesOverLimit:images.length>128,
      permittedImages:0, cssVisibleImages:0, inViewportImages:0, uniqueCandidates:0, opacityZeroImages:0, zeroSizeImages:0,
    };
    for (let index=0;index<Math.min(images.length,128);index++) {
      const image=images[index], url=mediaUrl(image.currentSrc||image.src);
      if(!url)continue;
      const parsed=new URL(url);
      if(parsed.hostname!=='pbs.twimg.com'||!/^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(parsed.pathname))continue;
      diagnostic.permittedImages++;
      if(getComputedStyle(image).opacity==='0')diagnostic.opacityZeroImages++;
      const rect=image.getBoundingClientRect();
      if(!(rect.width>0&&rect.height>0))diagnostic.zeroSizeImages++;
      if(!visible(image))continue;
      diagnostic.cssVisibleImages++;
      if(!(rect.width>0&&rect.height>0&&rect.right>0&&rect.bottom>0&&rect.left<window.innerWidth&&rect.top<window.innerHeight))continue;
      diagnostic.inViewportImages++;candidates.add(url);
    }
    diagnostic.uniqueCandidates=Math.min(candidates.size,2);
    result.diagnostic=diagnostic;
  }
  return result;
})()
