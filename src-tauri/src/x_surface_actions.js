// App-owned controls inside the original document; no private APIs or remote writes.
(() => {
  const config = __ATSUMI_X_ACTIONS__;
  if (window !== window.top || !['https://x.com','https://www.x.com'].includes(location.origin) || window.__atsumiXActions) return;
  // A missing native search handler is not a usable capability. Keep selection,
  // pixel capture and handoff dormant unless the injected build opts in.
  const searchEnabled = config.searchEnabled === true;
  const nativeSearch = config.searchCapabilities;
  const validSearchCapabilities = !!(nativeSearch && typeof nativeSearch === 'object' && !Array.isArray(nativeSearch)
    && Object.keys(nativeSearch).length === 2 && Object.hasOwn(nativeSearch,'image') && Object.hasOwn(nativeSearch,'video')
    && typeof nativeSearch.image === 'boolean' && typeof nativeSearch.video === 'boolean');
  const searchCapabilities = Object.freeze({image:validSearchCapabilities && nativeSearch.image === true,video:validSearchCapabilities && nativeSearch.video === true});
  const searchAllowed = kind => searchEnabled && (kind === 'image' ? searchCapabilities.image === true : kind === 'video' && searchCapabilities.video === true);
  let visible = false, menu = null, expansion = null, toast = null, photoControl = null, photoControlTarget = null;
  let photoRefresh = null;
  const pending = new Map(), finished = new Map(), notices = new Map();
  const searches = new Map();
  const controls = new WeakSet();
  const transport = () => {
    try {const bridge=window.chrome?.webview,send=bridge?.postMessage;return typeof send==='function'?send.bind(bridge):null;}catch{return null;}
  };
  const envelope=(kind,value)=>`https://x.com/__atsumi_surface__/${config.nonce}?kind=${encodeURIComponent(kind)}&value=${encodeURIComponent(value)}`;
  const signal = (kind,value,send=transport()) => {
    if (!visible||!send) return false;
    const encoded=envelope(kind,value);if(encoded.length>4096)return false;
    try{send(`ATSUMI_X_SURFACE_V1:${encoded}`);return true;}catch{return false;}
  };
  const postPath = value => {
    try {const u=new URL(value,location.href);return ['https://x.com','https://www.x.com'].includes(u.origin)&&/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}(?:\/(?:photo|video)\/[1-4])?$/.test(u.pathname)?u.pathname.replace(/\/(?:photo|video)\/[1-4]$/,''):null;}catch{return null;}
  };
  const own = (article,node) => node.closest('article[data-testid="tweet"]')===article
    && !node.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)');
  const mediaSelector='[data-testid="tweetPhoto"],[data-testid="videoPlayer"]';
  const mediaRoot = (article,node) => {
    let root=node.closest(mediaSelector);
    if(!root||!own(article,root))return null;
    for(let parent=root.parentElement?.closest(mediaSelector);parent&&own(article,parent);parent=root.parentElement?.closest(mediaSelector))root=parent;
    return root;
  };
  const pathFor = article => {
    if(!article.isConnected || window.__atsumiXAds?.blocked(article))return null;
    const paths=new Set([...article.querySelectorAll('[data-testid="User-Name"] time')].filter(n=>own(article,n)).map(n=>postPath(n.closest('a')?.href)).filter(Boolean));
    if(paths.size)return paths.size===1?[...paths][0]:null;
    // The main detail post puts its timestamp below the body, not in
    // User-Name. Admit that fallback only for the exact current detail route
    // and one unambiguous own timestamp, never a quote or inferred author.
    const detail=postPath(location.pathname);
    if(!detail)return null;
    const timestamps=new Set([...article.querySelectorAll('time')].filter(n=>own(article,n)).map(n=>postPath(n.closest('a')?.href)).filter(Boolean));
    return timestamps.size===1&&timestamps.has(detail)?detail:null;
  };
  const mediaFor = node => {
    const video=node.matches('video')?node:node.querySelector('video');
    const image=node.matches('img')?node:node.querySelector('img');
    const value=video?(video.poster||video.currentSrc||video.src):(image?.currentSrc||image?.src);
    try {const u=new URL(value);return u.protocol==='https:'&&['pbs.twimg.com','video.twimg.com'].includes(u.hostname)&&!u.port&&!u.username&&!u.password&&!u.hash&&u.href.length<=2048?u.href:null;}catch{return null;}
  };
  const displayed = node => {
    for(let n=node;n instanceof Element;n=n.parentElement){
      const style=getComputedStyle(n);
      if(n.hidden||n.hasAttribute('inert')||n.getAttribute('aria-hidden')==='true'
        ||style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse'||style.opacity==='0')return false;
    }
    return node instanceof Element;
  };
  const inViewport = r => r.width>0&&r.height>0&&r.right>0&&r.bottom>0&&r.left<innerWidth&&r.top<innerHeight;
  function photoPaint(image,media,trace) {
    const direct=displayed(image);
    if(trace){
      const rect=image.getBoundingClientRect();
      trace.imageLoaded+=image.naturalWidth>0&&image.naturalHeight>0?1:0;
      trace.imageComplete+=image.complete?1:0;
      trace.nonzeroRect+=rect.width>0&&rect.height>0?1:0;
      trace.inViewport+=inViewport(rect)?1:0;
      trace.directlyShown+=direct?1:0;
      trace.opacityZero+=getComputedStyle(image).opacity==='0'?1:0;
    }
    if(direct)return image;
    const style=getComputedStyle(image),parent=image.parentElement;
    // Match the archive reader's already supported transparent sizing image:
    // only an exact, visible, same-size literal background is presentation.
    if(style.opacity!=='0'||image.hidden||image.hasAttribute('inert')||image.getAttribute('aria-hidden')==='true'
      ||style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse'
      ||!parent||parent.tagName!=='DIV'||parent.children.length>8||!displayed(parent))return null;
    const rect=image.getBoundingClientRect();
    const matches=node=>{const r=node.getBoundingClientRect();return inViewport(r)&&['left','right','top','bottom'].every(k=>Math.abs(r[k]-rect[k])<=1);};
    if(!inViewport(rect)||!matches(parent))return null;
    const paints=[parent,...parent.children].filter(node=>node!==image&&getComputedStyle(node).backgroundImage&&getComputedStyle(node).backgroundImage!=='none');
    if(trace)trace.backgrounds=Math.min(128,trace.backgrounds+paints.length);
    if(paints.length!==1)return null;
    const paint=paints[0];
    if(paint!==parent&&(paint.tagName!=='DIV'||paint.childElementCount!==0||(paint.textContent||'').trim()))return null;
    if(!displayed(paint)||!matches(paint))return null;
    const bg=/^url\((?:"([^"\\]+)"|'([^'\\]+)'|([^"'()\\\s]+))\)$/.exec(getComputedStyle(paint).backgroundImage);
    const exact=!!bg&&(bg[1]||bg[2]||bg[3])===media;
    if(trace&&exact)trace.exactPaint++;
    return exact?paint:null;
  }
  function photoSlideShape(candidates) {
    // Debug-only structural evidence. This does not choose a slide or change
    // admission. No labels, URLs, selectors, dimensions or DOM objects escape.
    const shape={routeOrdinal:Number(location.pathname.match(/\/photo\/([1-4])$/)?.[1]||0),
      carousels:0,lists:0,slides:0,candidateSlides:0,indexedSlides:0,indexedRouteMatches:0,
      currentSlides:0,selectedSlides:0,currentRouteMatches:0,selectedRouteMatches:0,
      declaredSizes:0,matchingSetSizes:0,counterLabels:0,counterRouteMatches:0,counterSizeMatches:0,
      ordinalItemCandidates:0,fullyClipped:0,partlyClipped:0,unclipped:0,clipUncertain:0,overflow:false};
    const listSelector='[role="list"],ul,ol',itemSelector='[role="listitem"],li';
    const lists=new Set(),items=new Set(),carousels=new Set(),labels=new Set();
    const ordinal=value=>/^[1-4]$/.test(value||'')?Number(value):0;
    const carousel=node=>node?.getAttribute('role')==='carousel'||
      /^(carousel|회전 슬라이드|カルーセル)$/i.test(node?.getAttribute('aria-roledescription')||'');
    const boundedText=node=>{
      if(!(node instanceof Element)||node.childElementCount>4||node.querySelectorAll('*').length>8)return '';
      const value=(node.textContent||'').trim();return value.length<=80?value:'';
    };
    const counter=value=>{
      if(typeof value!=='string'||value.length>80)return null;
      const match=/^(?:slide\s+)?([1-4])\s*(?:\/|of)\s*([1-4])(?:\s*-\s*carousel)?$/i.exec(value.trim())
        ||/^슬라이드\s*([1-4])\s*\/\s*([1-4])\s*-\s*회전 슬라이드$/.exec(value.trim());
      return match&&Number(match[1])<=Number(match[2])?[Number(match[1]),Number(match[2])]:null;
    };
    const clip=image=>{
      const initial=image.getBoundingClientRect();
      let left=Math.max(initial.left,0),right=Math.min(initial.right,innerWidth),
        top=Math.max(initial.top,0),bottom=Math.min(initial.bottom,innerHeight),depth=0,node=image.parentElement;
      for(;node&&depth<32;node=node.parentElement,depth++){
        const style=getComputedStyle(node),rect=node.getBoundingClientRect();
        if(/^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowX||style.overflow)){
          left=Math.max(left,rect.left);right=Math.min(right,rect.right);
        }
        if(/^(hidden|clip|scroll|auto|overlay)$/.test(style.overflowY||style.overflow)){
          top=Math.max(top,rect.top);bottom=Math.min(bottom,rect.bottom);
        }
      }
      if(node){shape.clipUncertain++;return;}
      if(right<=left||bottom<=top)shape.fullyClipped++;
      else if(right-left<initial.width-1||bottom-top<initial.height-1)shape.partlyClipped++;
      else shape.unclipped++;
    };
    for(const candidate of candidates){
      clip(candidate.image);
      const item=candidate.image.closest(itemSelector),list=item?.parentElement?.closest(listSelector);
      if(item&&list&&item.closest(listSelector)===list){items.add(item);lists.add(list);}
      let node=candidate.image.parentElement,depth=0;
      for(;node&&depth<32;node=node.parentElement,depth++){
        if(carousel(node)){carousels.add(node);break;}
      }
      if(node&&depth===32)shape.overflow=true;
    }
    shape.carousels=Math.min(carousels.size,8);shape.lists=Math.min(lists.size,8);
    shape.candidateSlides=Math.min(items.size,8);
    if(carousels.size>8||lists.size>8||items.size>8){shape.overflow=true;return shape;}
    for(const list of lists){
      const slides=[...list.querySelectorAll(itemSelector)].filter(item=>item.closest(listSelector)===list);
      shape.slides=Math.min(8,shape.slides+slides.length);
      if(slides.length>4){shape.overflow=true;continue;}
      const selected=slides[shape.routeOrdinal-1];
      shape.ordinalItemCandidates+=candidates.filter(value=>value.image.closest(itemSelector)===selected).length;
      for(const item of slides){
        const position=ordinal(item.getAttribute('aria-posinset')),size=ordinal(item.getAttribute('aria-setsize'));
        const current=item.hasAttribute('aria-current')&&!['','false'].includes(item.getAttribute('aria-current'));
        const active=item.getAttribute('aria-selected')==='true';
        if(position){shape.indexedSlides++;if(position===shape.routeOrdinal)shape.indexedRouteMatches++;}
        if(size){shape.declaredSizes++;if(size===slides.length)shape.matchingSetSizes++;}
        if(current){shape.currentSlides++;if(position===shape.routeOrdinal)shape.currentRouteMatches++;}
        if(active){shape.selectedSlides++;if(position===shape.routeOrdinal)shape.selectedRouteMatches++;}
      }
    }
    for(const root of carousels){
      const roots=[root,root.previousElementSibling].filter(Boolean);
      // Only the current carousel's own label or immediately preceding
      // bounded status label: not arbitrary page text or an article counter.
      for(const node of roots){
        if(!displayed(node)||node.closest('article[data-testid="tweet"]'))continue;
        for(const value of [node.getAttribute('aria-label'),boundedText(node)]){
          const parsed=counter(value);if(!parsed)continue;
          const key=`${parsed[0]}/${parsed[1]}`;if(labels.has(key))continue;labels.add(key);
          shape.counterLabels++;
          if(parsed[0]===shape.routeOrdinal)shape.counterRouteMatches++;
          const matching=[...lists].filter(list=>root.contains(list));
          if(matching.length===1&&matching[0].querySelectorAll(itemSelector).length===parsed[1])shape.counterSizeMatches++;
        }
      }
    }
    return shape;
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
  function currentPhoto(trace=null) {
    const done=(reason,photo=null)=>{if(trace)trace.reason=reason;return photo;};
    if(!visible)return done('hidden');
    if(location.origin!=='https://x.com'||!/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(location.pathname))return done('route');
    let owner='';
    try{owner=window.__atsumiXSurface?.archivePhotoOwner?.();}catch{return done('owner');}
    if(typeof owner!=='string'||!/^[A-Za-z0-9_]{1,15}$/.test(owner))return done('owner');
    let scope=document;
    const dialogs=[...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')];
    if(trace){trace.dialogs=Math.min(128,dialogs.length);trace.articlePresent=!!document.querySelector('article[data-testid="tweet"]');}
    if(dialogs.length){
      const exposed=dialogs.filter(displayed);
      if(trace)trace.visibleDialogs=Math.min(128,exposed.length);
      if(exposed.length>8)return done('dialogLimit');
      // Nested modal shells are one layer. Two unrelated visible overlays are
      // ambiguous; do not guess a current photo from either or the background.
      const top=exposed.filter(node=>!exposed.some(other=>other!==node&&node.contains(other)));
      if(trace)trace.leafDialogs=Math.min(128,top.length);
      if(top.length!==1)return done('dialogAmbiguous');
      scope=top[0];
    }else if(document.querySelector('article[data-testid="tweet"]'))return done('articleScope');
    const images=scope.querySelectorAll('img'),candidates=new Map();
    if(trace)trace.scannedImages=Math.min(128,images.length);
    if(images.length>128)return done('imageLimit');
    for(const image of images){
      if(image.closest('article[data-testid="tweet"]'))continue;
      const media=mediaFor(image);if(!media)continue;
      const url=new URL(media);
      if(url.hostname!=='pbs.twimg.com'||!/^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(url.pathname)
        ||[...url.searchParams.keys()].some(k=>!['format','name'].includes(k)))continue;
      if(trace)trace.allowedImages++;
      const paint=photoPaint(image,media,trace);if(!paint||!inViewport(image.getBoundingClientRect()))continue;
      candidates.set(media,{image,paint,media,owner,scope,path:postPath(location.pathname),route:location.pathname});
    }
    if(trace){trace.candidates=candidates.size;trace.slideShape=photoSlideShape([...candidates.values()]);}
    const resolved=resolveCurrentCarouselPhoto([...candidates.values()],location.pathname,displayed,trace);
    if(resolved!==undefined)return resolved?done('ready',resolved):done(candidates.size?'ambiguous':'none');
    return candidates.size===1?done('ready',[...candidates.values()][0]):done(candidates.size?'ambiguous':'none');
  }
  function photoDiagnostic() {
    if(config.diagnostics!==true)return null;
    // This is the exact admission evaluator, not a second looser photo scan.
    // No source text, resource URL, DOM identity or raw CSS leaves this helper.
    const trace={reason:'hidden',dialogs:0,visibleDialogs:0,leafDialogs:0,articlePresent:false,
      scannedImages:0,allowedImages:0,imageLoaded:0,imageComplete:0,nonzeroRect:0,
      inViewport:0,directlyShown:0,opacityZero:0,backgrounds:0,exactPaint:0,candidates:0,
      controlPresent:!!photoControl?.isConnected,controlHovered:!!photoControl?.hasAttribute('data-media-hover')};
    currentPhoto(trace);
    return trace;
  }
  const samePhoto = (a,b) => !!a&&!!b&&a.image===b.image&&a.media===b.media&&a.path===b.path&&a.route===b.route&&a.owner===b.owner&&a.scope===b.scope;
  const collapse=()=>{
    if(!expansion)return;
    expansion.single.hidden=true;
    expansion.rail.removeAttribute('data-expanded');
    expansion.anchor.setAttribute('aria-expanded','false');
    expansion=null;
  };
  const closeMenu=()=>{const wasOpen=!!menu||!!expansion;menu?.remove();menu=null;collapse();return wasOpen;};
  const icon = name => {
    const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
    for(const [k,v] of Object.entries({viewBox:'0 0 24 24',fill:'none',stroke:'currentColor','stroke-width':'1.8','stroke-linecap':'round','stroke-linejoin':'round','aria-hidden':'true'}))svg.setAttribute(k,v);
    if(name==='download-all'){
      for(const transform of ['translate(1 1) scale(.68)','translate(7 7) scale(.68)']){
        const p=document.createElementNS(svg.namespaceURI,'path');p.setAttribute('d','M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4');p.setAttribute('transform',transform);svg.append(p);
      }
      return svg;
    }
    const p=document.createElementNS(svg.namespaceURI,'path');
    p.setAttribute('d',name==='save'?'M3 4h18v4H3zM5 8v12h14V8M10 12h4'
      :'M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4');svg.append(p);return svg;
  };
  const progressStages=new Set(['queued','capture','preparing','downloading','writing','complete','failed','cancelled']);
  const terminalStage=stage=>['complete','failed','cancelled'].includes(stage);
  const receiptStates=new Set(['active','complete','partial','failed','cancelled','absent']);
  const MAX_MISSED_RECEIPTS=15;
  const MAX_PENDING=22;
  function transferValue(value,received) {
    return value===null||value&&typeof value==='object'&&!Array.isArray(value)
      &&Object.keys(value).length===2&&Object.hasOwn(value,'receivedBytes')&&Object.hasOwn(value,'totalBytes')
      &&Number.isSafeInteger(value.receivedBytes)&&value.receivedBytes>=0&&value.receivedBytes<=received
      &&(value.totalBytes===null||Number.isSafeInteger(value.totalBytes)&&value.totalBytes>=value.receivedBytes);
  }
  function progressValue(value) {
    if(!value||typeof value!=='object'||Array.isArray(value))return null;
    const keys=['sequence','stage','processedMedia','totalMedia','downloadedBytes','totalBytes','partial','currentTransfer'];
    if(Object.keys(value).length!==keys.length||!keys.every(key=>Object.hasOwn(value,key))
      ||!progressStages.has(value.stage)||typeof value.partial!=='boolean'
      ||!['sequence','processedMedia','totalMedia','downloadedBytes'].every(key=>Number.isSafeInteger(value[key])&&value[key]>=0)
      ||value.processedMedia>value.totalMedia||value.totalMedia>4096
      ||value.totalBytes!==null&&(!Number.isSafeInteger(value.totalBytes)||value.totalBytes<value.downloadedBytes)
      ||!transferValue(value.currentTransfer,value.downloadedBytes))return null;
    return {...value,currentTransfer:value.currentTransfer?{...value.currentTransfer}:null};
  }
  const sameProgress=(a,b)=>Object.keys(a).every(key=>key==='currentTransfer'
    ?a[key]===null?b[key]===null:b[key]!==null&&a[key].receivedBytes===b[key].receivedBytes&&a[key].totalBytes===b[key].totalBytes
    :a[key]===b[key]);
  function articleBinding(article,path,root=null) {
    const roots=root?[root]:[...article.querySelectorAll(mediaSelector)].filter(node=>mediaRoot(article,node)===node).slice(0,16);
    const targets=roots.map(node=>({node,media:mediaFor(node)})).filter(target=>target.media);
    return {current:()=>pathFor(article)===path&&(!root||targets.length===1&&root.isConnected
      &&mediaRoot(article,root)===root&&mediaFor(root)===targets[0].media),targets:targets.map(target=>({
      node:target.node,current:()=>pathFor(article)===path&&target.node.isConnected
        &&mediaRoot(article,target.node)===target.node&&mediaFor(target.node)===target.media,
    }))};
  }
  function photoBinding(photo) {
    return {current:()=>samePhoto(photo,currentPhoto()),targets:[{node:photo.image,photo:true,current:()=>samePhoto(photo,currentPhoto())}]};
  }
  const jobCurrent=job=>visible&&location.href===job.href&&job.binding?.current()===true;
  const clearLines=job=>{for(const line of job.lines)line?.remove();job.lines=[];};
  function progressText(job) {
    const value=job.progress;
    if(job.unknown)return '저장 상태 확인 필요 · 응답을 기다리는 중';
    if(job.cancelling&&!terminalStage(value.stage))return '취소 요청 확인 중…';
    if(!job.accepted)return '요청 확인 중…';
    if(job.lastSequence<0&&!terminalStage(value.stage))return '저장 요청이 접수되었습니다.';
    if(value.stage==='queued')return '대기 중';
    if(value.stage==='capture')return '저장할 내용을 확인하는 중…';
    if(value.stage==='preparing')return '저장 준비 중…';
    if(value.stage==='writing')return '저장 마무리 중…';
    if(value.stage==='failed')return '저장 실패';
    if(value.stage==='cancelled')return '저장 취소됨';
    if(value.stage==='complete')return value.partial?'일부만 저장됨':'저장 완료';
    const count=value.totalMedia?`${value.processedMedia}/${value.totalMedia}`:'';
    const bytes=`받은 ${(value.downloadedBytes/1048576).toFixed(1)} MB`;
    return [count,bytes].filter(Boolean).join(' · ');
  }
  function clearTextTimer(job,reset=false) {
    if(job.textTimer)clearTimeout(job.textTimer.id);
    job.textTimer=null;if(reset)job.displayText=null;
  }
  function noticeTextMode(job) {
    return job.unknown?'unknown':job.cancelling&&!terminalStage(job.progress.stage)?'cancelling'
      :!job.accepted?'awaiting':job.lastSequence<0&&!terminalStage(job.progress.stage)?'accepted'
      :job.progress.stage==='complete'&&job.progress.partial?'partial':job.progress.stage;
  }
  function stableNoticeText(job) {
    const mode=noticeTextMode(job),text=progressText(job),now=Date.now(),previous=job.displayText;
    // Only downloading numbers are coalesced. Truth/phase changes and native
    // progress bars stay immediate, including uncertainty and cancellation.
    if(!previous||previous.mode!==mode||mode!=='downloading'){
      clearTextTimer(job);job.displayText={mode,text,at:now};return text;
    }
    if(previous.text===text)return text;
    const remaining=750-(now-previous.at);
    if(remaining<=0){clearTextTimer(job);job.displayText={mode,text,at:now};return text;}
    if(!job.textTimer){
      const ticket={id:null};job.textTimer=ticket;
      ticket.id=setTimeout(()=>{
        if(job.textTimer!==ticket||!visible||pending.get(job.id)!==job)return;
        job.textTimer=null;
        job.displayText={mode:noticeTextMode(job),text:progressText(job),at:Date.now()};
        paintJob(job);
      },remaining);
    }
    return previous.text;
  }
  const setAttribute=(node,key,value)=>{if(node.getAttribute(key)!==value)node.setAttribute(key,value);};
  const setStyle=(node,key,value)=>{if(node?.style&&node.style[key]!==value)node.style[key]=value;};
  function renderProgress(job) {
    const current=jobCurrent(job);
    if(!visible){clearTextTimer(job,true);clearLines(job);removeNotice(job.id);return;}
    if(!current)clearLines(job);
    const value=job.progress,state=job.unknown?'unknown':!job.accepted?'awaiting':value.stage==='complete'&&value.partial?'partial':value.stage;
    const percent=job.unknown||!job.accepted?null:value.stage==='complete'?100:value.totalBytes>0?Math.min(99,100*value.downloadedBytes/value.totalBytes):null;
    if(current&&job.kind!=='archive'){
      for(let i=0;i<job.binding.targets.length;i++){
        const target=job.binding.targets[i];let line=job.lines[i];
        if(!target.current()){line?.remove();continue;}
        if(line?.isConnected&&(line.parentElement!==(target.photo?document.body:target.node)||line.childElementCount!==1||line.firstElementChild?.tagName!=='SPAN')){line.remove();line=null;}
        if(!line?.isConnected){
          line=document.createElement('div');line.className='atsumi-x-media-progress';line.setAttribute('role','progressbar');line.setAttribute('aria-label',job.kind==='all'?'게시물 미디어 전체 저장 진행':'선택 미디어 저장 진행');
          line.dataset.request=job.id;
          line.append(document.createElement('span'));job.lines[i]=line;
          if(target.photo){line.dataset.atsumiPhotoProgress='true';document.body.append(line);}else target.node.append(line);
        }
        setAttribute(line,'data-state',state);setAttribute(line,'aria-valuetext',progressText(job));
        if(percent===null){line.removeAttribute('aria-valuenow');if(job.unknown||terminalStage(value.stage))line.removeAttribute('data-indeterminate');else setAttribute(line,'data-indeterminate','true');}
        else{setAttribute(line,'aria-valuenow',String(Math.floor(percent)));line.removeAttribute('data-indeterminate');}
        setStyle(line.firstElementChild,'width',percent===null?'30%':`${percent}%`);
        if(!target.photo)setStyle(line,'bottom',`${Math.max(0,[...target.node.querySelectorAll(':scope > .atsumi-x-media-progress')].indexOf(line))*3}px`);
        if(target.photo){const r=target.node.getBoundingClientRect();setStyle(line,'left',`${Math.max(0,r.left)}px`);setStyle(line,'top',`${Math.min(innerHeight-3,r.bottom-3)}px`);setStyle(line,'width',`${Math.max(0,Math.min(innerWidth,r.right)-Math.max(0,r.left))}px`);}
      }
    }
    const row=notices.get(job.id);
    if(pending.get(job.id)===job&&row?.isConnected){
      const cancel=row.querySelector('button');if(cancel){cancel.disabled=job.cancelAwaiting||terminalStage(value.stage);cancel.textContent=job.cancelling&&!job.cancelAwaiting?'취소 다시 요청':'취소';}
      const check=row.querySelector('[data-atsumi-save-status]');if(check)check.hidden=job.misses<MAX_MISSED_RECEIPTS;
      const text=row.querySelector('[data-atsumi-notice-text]'),label=stableNoticeText(job);if(text&&text.textContent!==label)text.textContent=label;
      const meter=row.querySelector('[data-atsumi-notice-progress]');
      if(meter){setAttribute(meter,'data-state',state);setAttribute(meter,'aria-valuetext',label);
        if(percent===null){meter.removeAttribute('aria-valuenow');if(job.unknown||terminalStage(value.stage))meter.removeAttribute('data-indeterminate');else setAttribute(meter,'data-indeterminate','true');}else{setAttribute(meter,'aria-valuenow',String(Math.floor(percent)));meter.removeAttribute('data-indeterminate');}
        setStyle(meter.firstElementChild,'width',percent===null?'30%':`${percent}%`);}
    }
  }
  function reconcileProgress() {
    for(const job of [...pending.values(),...finished.values()]){
      paintJob(job);
    }
  }
  function paintJob(job) {
    // Source DOM is mutable. A removed/replaced UI child must never prevent
    // request submission, acknowledgement, terminal cleanup or status polling.
    try{
      if(visible&&pending.get(job.id)===job&&!notices.get(job.id)?.isConnected)notice(job.id,stableNoticeText(job),true);
      renderProgress(job);
    }catch{} // Job truth is native; rendering failure never invents an outcome.
  }
  function scheduleReceipt(job) {
    if(!visible||pending.get(job.id)!==job||job.poll||job.misses>=MAX_MISSED_RECEIPTS)return;
    const ticket={};job.poll=ticket;
    setTimeout(()=>{
      if(job.poll!==ticket||pending.get(job.id)!==job)return;job.poll=null;
      if(!visible)return;
      job.misses++;
      if(job.misses>=3)job.unknown=true;
      if(!signal('archiveStatus',job.id))job.unknown=true;
      paintJob(job);scheduleReceipt(job);
    },2000);
  }
  function confirmed(job) {
    job.accepted=true;job.unknown=false;job.misses=0;
    scheduleReceipt(job);
  }
  function acceptProgress(job,value) {
    const next=progressValue(value);if(!next)return false;
    if(job.terminal||next.sequence<=job.lastSequence)return false;
    confirmed(job);
    job.lastSequence=next.sequence;job.progress=next;job.terminal=terminalStage(next.stage);
    return true;
  }
  function finishJob(id,state,message) {
    const job=pending.get(id);if(!job)return;clearTextTimer(job,true);pending.delete(id);job.poll=null;job.accepted=true;job.unknown=false;
    const complete=['done','complete','partial'].includes(state);
    job.progress={...job.progress,stage:complete?'complete':state==='cancelled'?'cancelled':'failed',partial:state==='partial'||job.progress.partial};job.terminal=true;
    if(state==='absent')clearLines(job);
    else{finished.set(id,job);paintJob(job);setTimeout(()=>{if(finished.get(id)===job){clearLines(job);finished.delete(id);removeNotice(id);}},8000);}
    while(finished.size>MAX_PENDING){const [oldId,oldJob]=finished.entries().next().value;clearLines(oldJob);finished.delete(oldId);removeNotice(oldId);}
    if(visible){try{notice(id,String(message).slice(0,512),false,(state==='done'||state==='complete')&&!job.progress.partial);}catch{}}
    else{clearLines(job);removeNotice(id);}
  }
  function syncNotice() {
    if(!toast)return;
    if(!toast.childElementCount){toast.remove();toast=null;return;}
    // Stable insertion order, never the most recently reporting request.
    toast.dataset.request=toast.firstElementChild.dataset.request;
  }
  function removeNotice(id) {notices.get(id)?.remove();notices.delete(id);syncNotice();}
  function notice(id,message,running=false,success=false) {
    if(!toast?.isConnected){toast=document.createElement('div');toast.className='atsumi-x-save-notice';toast.setAttribute('role','region');toast.setAttribute('aria-label','다운로드 진행');document.body.append(toast);}
    let row=notices.get(id);
    if(row?.isConnected&&running)return;
    if(!row?.isConnected){row=document.createElement('div');row.className='atsumi-x-save-row';row.dataset.request=id;row.setAttribute('role','status');notices.set(id,row);toast.append(row);}
    else row.replaceChildren();
    row.setAttribute('role',running?'group':'status');
    const job=pending.get(id)||finished.get(id);
    if(job){const target=document.createElement('span');target.dataset.atsumiNoticeTarget='true';target.textContent=`@${job.path.split('/')[1]} · ${job.kind==='one'?'개별':job.kind==='all'?'전체':'게시물 보관'} · `;target.title=`https://x.com${job.path}`;row.append(target);}
    if(success){
      const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('class','atsumi-x-save-check');svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('role','img');svg.setAttribute('aria-label','저장 완료');
      const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d','M4 12.5 9 18 20 6');path.setAttribute('pathLength','1');svg.append(path);row.append(svg);
    }else{const text=document.createElement('span');text.dataset.atsumiNoticeText='true';if(running)text.setAttribute('role','status');text.textContent=message;row.append(text);}
    if(running){
      const cancel=document.createElement('button');controls.add(cancel);cancel.type='button';cancel.textContent='취소';cancel.disabled=pending.get(id)?.cancelAwaiting===true;
      cancel.onclick=e=>{
        const job=pending.get(id);if(!e.isTrusted||!visible||!job||job.cancelAwaiting||job.terminal)return;
        job.cancelling=true;job.cancelAwaiting=true;cancel.disabled=true;
        if(!signal('archiveCancel',id))job.unknown=true;
        paintJob(job);scheduleReceipt(job);
      };
      row.append(cancel);
      const check=document.createElement('button');controls.add(check);check.type='button';check.textContent='상태 확인';check.dataset.atsumiSaveStatus='true';check.hidden=true;
      check.onclick=e=>{const job=pending.get(id);if(!e.isTrusted||!visible||!job||job.poll)return;job.misses=0;job.unknown=true;signal('archiveStatus',id);paintJob(job);scheduleReceipt(job);};row.append(check);
      const meter=document.createElement('div');meter.className='atsumi-x-media-progress';meter.dataset.atsumiNoticeProgress='true';meter.setAttribute('role','progressbar');meter.setAttribute('aria-label','보관 진행');meter.append(document.createElement('span'));row.append(meter);
    }
    syncNotice();
    if(!running){const owned=row;setTimeout(()=>{if(notices.get(id)===owned)removeNotice(id);},8000);}
  }
  function start(path,kind,media=null,binding=null) {
    if(!visible||!postPath(path)||!binding?.current())return;
    const duplicate=[...pending.values()].find(job=>job.path===path&&job.kind===kind&&job.media===media);
    if(duplicate){
      const job=duplicate;
      paintJob(job);scheduleReceipt(job);
      try{const row=notices.get(job.id);if(row&&!row.querySelector('[data-atsumi-save-busy]')){const hint=document.createElement('span');hint.dataset.atsumiSaveBusy='true';hint.textContent=' · 이미 요청됨';row.append(hint);}}catch{}
      return;
    }
    if(pending.size>=MAX_PENDING){try{notice('capacity','요청이 가득 찼습니다. 진행 중인 작업을 확인해 주세요.');}catch{}return;}
    const send=transport();
    if(!send){try{notice('unsent','저장 요청을 보내지 못했습니다. 앱 연결을 확인해 주세요.');}catch{}return;}
    const id=crypto.randomUUID();
    const submission=JSON.stringify({id,path,kind,media});
    if(envelope('archive',submission).length>4096){try{notice('unsent','저장 요청을 보내지 못했습니다. 미디어 정보가 허용 길이를 초과했습니다.');}catch{}return;}
    for(const [oldId,oldJob]of finished)if(oldJob.path===path&&oldJob.kind===kind&&oldJob.media===media){try{clearLines(oldJob);removeNotice(oldId);}catch{}finished.delete(oldId);}
    const job={id,kind,path,media,cancelling:false,cancelAwaiting:false,href:location.href,binding,lines:[],lastSequence:-1,terminal:false,accepted:false,unknown:false,misses:0,poll:null,
      progress:{sequence:0,stage:'capture',processedMedia:0,totalMedia:0,downloadedBytes:0,totalBytes:null,partial:false,currentTransfer:null}};
    pending.set(id,job);try{closeMenu();}catch{}
    paintJob(job);
    // Captured transport proves a missing bridge never submitted anything.
    // A throwing postMessage may already have delivered: retain and reconcile.
    if(!signal('archive',submission,send)&&pending.get(id)===job){job.unknown=true;paintJob(job);}
    scheduleReceipt(job);
  }
  function item(label,run) {
    const b=document.createElement('button');controls.add(b);b.type='button';b.setAttribute('role','menuitem');b.textContent=label;
    b.onclick=e=>{if(!e.isTrusted)return;e.preventDefault();e.stopPropagation();e.stopImmediatePropagation();closeMenu();run();};return b;
  }
  const searchImageUrl = value => {
    try{const url=new URL(value);return url.protocol==='https:'&&url.hostname==='pbs.twimg.com'&&!url.username&&!url.password&&!url.port&&!url.hash
      &&url.href.length<=2048&&/^\/media\/[A-Za-z0-9_-]+(?:\.(?:jpg|jpeg|png|webp|gif))?$/.test(url.pathname)
      &&[...url.searchParams.keys()].every(key=>['format','name'].includes(key))?url.href:null;}catch{return null;}
  };
  function clearSearch(id) {
    const entry=searches.get(id);if(!entry)return;
    clearTimeout(entry.expire);clearTimeout(entry.cleanup);entry.payload=null;searches.delete(id);
  }
  const searchNotice=message=>{try{notice('search-notice',message);}catch{}};
  function searchMediaRoot(article,node) {
    const scope=node.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)')||article;
    const video=node.closest('[data-testid="videoPlayer"]'),photo=node.closest('[data-testid="tweetPhoto"]');
    const root=video&&scope.contains(video)?video:photo&&scope.contains(photo)?photo:null;
    return root?.closest('article[data-testid="tweet"]')===article?root:null;
  }
  function sourceSearchTarget(article,path,root,target) {
    if(!searchEnabled||!root)return null;
    const scope=root.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)')||article;
    const current=()=>visible&&pathFor(article)===path&&root.isConnected&&root.closest('article[data-testid="tweet"]')===article
      &&(root.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)')||article)===scope;
    const videos=[...root.querySelectorAll('video')].filter(video=>searchMediaRoot(article,video)===root);
    if(videos.length){
      if(videos.length!==1)return null;
      const video=videos[0],source=video.currentSrc||video.src;
      return {kind:'video',node:video,current:()=>current()&&video.isConnected&&searchMediaRoot(article,video)===root&&(video.currentSrc||video.src)===source&&displayed(video)&&inViewport(video.getBoundingClientRect())};
    }
    // An actual image target is stronger than a container's first descendant.
    // Never borrow a poster, avatar or a neighbouring image in a grouped root.
    const images=[...root.querySelectorAll('img')].filter(image=>searchMediaRoot(article,image)===root&&searchImageUrl(image.currentSrc||image.src));
    const direct=target?.closest('img'),image=images.includes(direct)?direct:images.length===1?images[0]:null;
    const imageUrl=image&&searchImageUrl(image.currentSrc||image.src);if(!imageUrl)return null;
    return {kind:'image',node:image,imageUrl,current:()=>current()&&image.isConnected&&searchMediaRoot(article,image)===root
      &&searchImageUrl(image.currentSrc||image.src)===imageUrl&&!!photoPaint(image,imageUrl)&&inViewport(image.getBoundingClientRect())};
  }
  function beginSearch(path,target) {
    if(!searchEnabled)return;
    if(!searchAllowed(target?.kind)){searchNotice('검색 서비스 연결 검증이 아직 끝나지 않았습니다. 미디어는 전송하지 않았습니다.');return;}
    if(!target?.current()||!postPath(path)){searchNotice('검색할 미디어가 변경되었거나 보이지 않습니다.');return;}
    if(searches.size>=4){searchNotice('앞선 검색 요청을 처리하고 있습니다. 잠시 후 다시 시도해 주세요.');return;}
    const id=crypto.randomUUID(),kind=target.kind,provider=kind==='image'?'saucenao':'lens',sourceUrl=location.href;
    let payload={id,kind,provider,path,sourceUrl};
    if(kind==='image')payload={...payload,imageUrl:target.imageUrl};
    else{
      const video=target.node,width=video.videoWidth,height=video.videoHeight,time=video.currentTime;
      if(video.mediaKeys){searchNotice('보호된 영상의 장면은 검색할 수 없습니다.');return;}
      if(video.readyState<2||!Number.isFinite(time)||time<0||!Number.isSafeInteger(width)||!Number.isSafeInteger(height)
        ||width<1||height<1||width>16384||height>16384||width*height>67108864){searchNotice('현재 영상 장면을 읽을 수 없습니다. 영상이 표시된 후 다시 시도해 주세요.');return;}
      const scale=Math.min(1,2048/Math.max(width,height)),canvas=document.createElement('canvas');
      canvas.width=Math.max(1,Math.round(width*scale));canvas.height=Math.max(1,Math.round(height*scale));
      try{
        const context=canvas.getContext('2d');if(!context)throw new Error('canvas');
        context.drawImage(video,0,0,canvas.width,canvas.height);
        const dataUrl=canvas.toDataURL('image/jpeg',.9),match=/^data:image\/jpeg;base64,([A-Za-z0-9+/]+={0,2})$/.exec(dataUrl);
        if(!match||match[1].length%4!==0||match[1].length/4*3-(match[1].endsWith('==')?2:match[1].endsWith('=')?1:0)>4*1024*1024)throw new Error('size');
        payload={...payload,dataUrl,width:canvas.width,height:canvas.height,time};
      }catch{searchNotice('이 영상의 현재 장면에 접근할 수 없습니다. 이미지는 전송하지 않았습니다.');return;}
      finally{canvas.width=0;canvas.height=0;}
    }
    if(!target.current()||sourceUrl!==location.href){searchNotice('검색할 미디어가 변경되었습니다. 다시 선택해 주세요.');return;}
    const entry={payload,current:target.current,sourceUrl,expires:Date.now()+10000,expire:null,cleanup:null};
    searches.set(id,entry);
    entry.expire=setTimeout(()=>{if(searches.get(id)===entry&&entry.payload){clearSearch(id);if(visible&&location.href===sourceUrl)searchNotice('검색 요청을 확인하지 못했습니다. 다시 선택해 주세요.');}},10000);
    entry.cleanup=setTimeout(()=>{if(searches.get(id)===entry)clearSearch(id);},120000);
    if(!signal('search',JSON.stringify({id,kind,provider,path}))){clearSearch(id);searchNotice('검색 요청을 전달하지 못했습니다. 다시 시도해 주세요.');}
    else searchNotice('검색할 미디어를 확인하는 중…');
  }
  function searchItem(path,target) {
    const label=target.kind==='image'?'작가·원본 찾기':'현재 장면으로 검색';
    const button=item(label,()=>beginSearch(path,target));
    button.title=target.kind==='image'?'SauceNAO · 선택한 이미지 주소를 전달합니다.':'Google Lens · 현재 영상 장면을 전송합니다.';
    button.setAttribute('aria-description',button.title);return button;
  }
  function placeMenu(anchor,point) {
    document.body.append(menu);
    const r=anchor?.getBoundingClientRect();const x=point?.x??r?.right??8,y=point?.y??r?.bottom??8;
    const width=menu.offsetWidth||232;
    menu.style.left=`${Math.max(8,Math.min(x-width+12,innerWidth-width-8))}px`;
    menu.style.top=`${Math.max(8,Math.min(y,innerHeight-menu.offsetHeight-8))}px`;
  }
  function choices(anchor,article,path,root,point,target=null) {
    const media=root?mediaFor(root):null;
    // Virtualized tweets can reuse the same elements while a menu is open.
    // Never submit an earlier post or media selection after that target changes.
    const current=()=>visible&&pathFor(article)===path&&(!root
      ||root.isConnected&&mediaRoot(article,root)===root&&mediaFor(root)===media);
    window.__atsumiXSurface?.dismissMenu?.();
    closeMenu();menu=document.createElement('div');menu.className='atsumi-x-download-menu';menu.setAttribute('role','menu');menu.setAttribute('aria-label','게시물 메뉴');
    if(media)menu.append(item('개별',()=>{if(current())start(path,'one',media,articleBinding(article,path,root));}));
    menu.append(item('전체',()=>{if(current())start(path,'all',null,articleBinding(article,path));}));
    if(point){
      const search=searchEnabled&&sourceSearchTarget(article,path,target?searchMediaRoot(article,target):root,target);if(search)menu.append(searchItem(path,search));
      menu.append(item('게시물 보관',()=>{if(current())start(path,'archive',null,articleBinding(article,path));}));
      menu.append(item('본문·댓글 선택 보관',()=>{if(current())signal('archiveChoose',path);}));
      menu.append(item('외부 브라우저에서 열기',()=>{if(current())signal('external',path);}));
    }
    placeMenu(anchor,point);
  }
  function photoChoices(anchor,photo,point) {
    const current=()=>samePhoto(photo,currentPhoto());
    window.__atsumiXSurface?.dismissMenu?.();
    closeMenu();menu=document.createElement('div');menu.className='atsumi-x-download-menu';menu.setAttribute('role','menu');menu.setAttribute('aria-label','사진 메뉴');
    menu.append(item('개별',()=>{if(current())start(photo.path,'one',photo.media,photoBinding(photo));}));
    if(searchEnabled&&point&&searchImageUrl(photo.media))menu.append(searchItem(photo.path,{kind:'image',imageUrl:photo.media,current}));
    if(point)menu.append(item('외부 브라우저에서 열기',()=>{if(current())signal('external',photo.route);}));
    placeMenu(anchor,point);
  }
  function button(name,label) {
    const b=document.createElement('button');controls.add(b);b.type='button';b.className='atsumi-x-inline-action';b.setAttribute('aria-label',label);b.title=label;b.append(icon(name));return b;
  }
  function expand(rail,anchor,single,current) {
    closeMenu();
    if(!current())return;
    expansion={rail,anchor,single,current};
    single.hidden=false;rail.setAttribute('data-expanded','true');anchor.setAttribute('aria-expanded','true');
  }
  function install(article,path) {
    const group=[...article.querySelectorAll('[role="group"]')].find(n=>own(article,n)&&n.querySelector('[data-testid="like"],[data-testid="unlike"],[data-testid="bookmark"],[data-testid="removeBookmark"]'));
    if(group&&!group.querySelector('[data-atsumi-post-save]')){
      const b=button('save','게시물 보관');b.dataset.atsumiPostSave='true';
      b.onclick=e=>{if(!e.isTrusted||!visible)return;e.preventDefault();e.stopImmediatePropagation();const current=pathFor(article);if(current&&b.isConnected&&own(article,b))start(current,'archive',null,articleBinding(article,current));};
      group.append(b);
    }
    // X hydrates poster/photos into nested video players. Both selectors can
    // describe the same physical media, so only its outer boundary owns a
    // control. Reconcile earlier controls when that boundary changes in place.
    const roots=[...article.querySelectorAll(mediaSelector)].filter(n=>mediaRoot(article,n)===n).slice(0,16),admitted=new Set(roots),seen=new Set();
    for(const rail of article.querySelectorAll('[data-atsumi-media-rail]')){
      if(rail.closest('article[data-testid="tweet"]')!==article)continue;
      const host=rail.parentElement;
      if(!admitted.has(host)||seen.has(host)||!controls.has(rail.querySelector('[data-atsumi-media-save]'))
        ||!controls.has(rail.querySelector('[data-atsumi-single-save]'))){
        if(expansion?.rail===rail)collapse();rail.remove();
      }else seen.add(host);
    }
    for(const host of article.querySelectorAll('.atsumi-x-media-host')){
      if(host.closest('article[data-testid="tweet"]')===article&&!admitted.has(host)){
        host.classList.remove('atsumi-x-media-host');host.removeAttribute('data-atsumi-relative-host');
      }
    }
    for(const root of roots.slice(0,16)){
      // A native tweetPhoto is often absolutely positioned. Forcing it to
      // relative collapses its sizing/paint layer and produces a black image.
      root.classList.add('atsumi-x-media-host');
      const position=getComputedStyle(root).position;
      if(position==='static')root.setAttribute('data-atsumi-relative-host','true');
      else if(position!=='relative')root.removeAttribute('data-atsumi-relative-host');
      if(root.querySelector(':scope > [data-atsumi-media-rail]'))continue;
      // Only app-owned controls are wrapped. The native image/player and its
      // action row are never moved or resized. Keeping the all button last
      // anchors it at the same right edge as the second icon expands left.
      const rail=document.createElement('div');rail.className='atsumi-x-download-rail';rail.dataset.atsumiMediaRail='true';rail.setAttribute('role','toolbar');rail.setAttribute('aria-label','미디어 다운로드');
      const b=button('download-all','전체');b.dataset.atsumiMediaSave='true';b.setAttribute('aria-expanded','false');
      const single=button('download','개별');single.dataset.atsumiSingleSave='true';single.hidden=true;
      const bound=()=>rail.isConnected&&b.parentElement===rail&&single.parentElement===rail&&rail.parentElement===root&&mediaRoot(article,root)===root;
      b.onclick=e=>{if(!e.isTrusted||!visible)return;e.preventDefault();e.stopImmediatePropagation();const current=pathFor(article);if(current&&bound())start(current,'all',null,articleBinding(article,current));};
      single.onclick=e=>{
        if(!e.isTrusted||!visible)return;e.preventDefault();e.stopImmediatePropagation();
        const active=expansion;if(active?.rail===rail&&active.current())active.run();
      };
      const show=e=>{
        if(!e.isTrusted||!visible)return;
        const path=pathFor(article),media=mediaFor(root);if(!path||!media||!bound())return;
        const current=()=>visible&&bound()&&pathFor(article)===path&&mediaFor(root)===media;
        expand(rail,b,single,current);
        if(expansion?.rail===rail)expansion.run=()=>start(path,'one',media,articleBinding(article,path,root));
      };
      const leave=e=>{
        if(!e.isTrusted||expansion?.rail!==rail||e.relatedTarget instanceof Node&&rail.contains(e.relatedTarget))return;
        collapse();
      };
      b.addEventListener('pointerenter',show);b.addEventListener('focus',show);
      rail.addEventListener('pointerleave',leave);rail.addEventListener('focusout',leave);
      rail.append(single,b);root.append(rail);
    }
  }
  function queuePhotoRefresh(event) {
    if(!event.isTrusted||!visible||photoRefresh||
      !/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(location.pathname))return;
    if(event.type!=='resize'){
      const target=event.target;
      if(!(target instanceof Element)||target.closest('article[data-testid="tweet"]')||
        (event.type==='load'?!target.matches('img'):!target.closest('[role="carousel"],[aria-roledescription]')))return;
    }
    const owner=window.__atsumiXSurface?.archivePhotoOwner?.();
    if(typeof owner!=='string'||!/^[A-Za-z0-9_]{1,15}$/.test(owner))return;
    const request={owner,route:location.pathname};photoRefresh=request;
    setTimeout(()=>{
      if(photoRefresh!==request)return;photoRefresh=null;
      if(!visible||location.pathname!==request.route||window.__atsumiXSurface?.archivePhotoOwner?.()!==request.owner)return;
      update();
    },0);
  }
  document.addEventListener('load',queuePhotoRefresh,true);
  document.addEventListener('transitionend',queuePhotoRefresh,true);
  window.addEventListener?.('resize',queuePhotoRefresh);
  function update() {
    photoRefresh=null;
    if(!visible||!installStyle())return;
    reconcileProgress();
    if(expansion&&!expansion.current())collapse();
    for(const article of [...document.querySelectorAll('article[data-testid="tweet"]')].slice(0,100)){
      const path=pathFor(article);if(path)install(article,path);
    }
    const photo=currentPhoto();
    if(!photo){photoControl?.remove();photoControl=null;photoControlTarget=null;return;}
    if(!photoControl?.isConnected){
      photoControl=button('download','개별');photoControl.dataset.atsumiPhotoSave='true';photoControl.classList.add('atsumi-x-photo-action');
      photoControl.onclick=e=>{
        if(!e.isTrusted||!visible)return;e.preventDefault();e.stopImmediatePropagation();
        const current=currentPhoto();if(current)start(current.path,'one',current.media,photoBinding(current));
      };
      document.body.append(photoControl);
    }
    const rect=photo.image.getBoundingClientRect();
    if(!samePhoto(photoControlTarget,photo))photoControl.removeAttribute('data-media-hover');
    photoControlTarget=photo;
    photoControl.style.left=`${Math.max(8,Math.min(rect.right-40,innerWidth-40))}px`;
    photoControl.style.top=`${Math.max(8,Math.min(rect.top+8,innerHeight-40))}px`;
  }
  const style=document.createElement('style');
  style.textContent='.atsumi-x-inline-action{display:grid!important;place-items:center!important;width:32px!important;height:32px!important;min-width:32px!important;padding:0!important;border:0!important;border-radius:50%!important;color:#e7e9ea!important;background:transparent!important;cursor:pointer!important;flex:none!important}.atsumi-x-inline-action svg{display:block!important;width:19px!important;height:19px!important;pointer-events:none}.atsumi-x-inline-action:hover{background:#1d9bf033!important;color:#1d9bf0!important}:where(.atsumi-x-media-host[data-atsumi-relative-host]){position:relative}.atsumi-x-media-host>[data-atsumi-media-save]{position:absolute!important;top:8px!important;right:8px!important;z-index:5!important;background:#000b!important}.atsumi-x-download-menu{position:fixed;z-index:2147483647;box-sizing:border-box;width:232px;max-width:calc(100vw - 16px);max-height:calc(100vh - 16px);overflow-y:auto;padding:4px;background:#000;color:#e7e9ea;border:1px solid #2f3336;border-radius:12px;box-shadow:0 0 12px #ffffff30;font:14px system-ui}.atsumi-x-download-menu button{display:block;width:100%;padding:12px;border:0;background:transparent;color:inherit;text-align:left;font:inherit;cursor:pointer}.atsumi-x-download-menu button:hover{background:#16181c}.atsumi-x-save-notice{position:fixed;z-index:2147483646;bottom:20px;left:50%;transform:translateX(-50%);max-width:calc(100vw - 32px);padding:12px 18px;background:#1d9bf0;color:#fff;border-radius:8px;font:14px system-ui}.atsumi-x-save-notice button{margin-left:12px;border:0;background:transparent;color:inherit;text-decoration:underline;cursor:pointer}';
  style.textContent+='.atsumi-x-photo-action{position:fixed!important;z-index:2147483645!important;background:#000b!important;opacity:0;pointer-events:none}.atsumi-x-photo-action[data-media-hover],.atsumi-x-photo-action:focus{opacity:1;pointer-events:auto}';
  style.textContent+='.atsumi-x-download-menu button{box-sizing:border-box;border-radius:8px}.atsumi-x-download-menu button:focus-visible{background:#16181c;outline:2px solid #1d9bf0;outline-offset:-2px}';
  style.textContent+='.atsumi-x-media-progress{position:absolute;left:0;right:0;bottom:0;height:3px;overflow:hidden;background:#1d9bf033;pointer-events:none;z-index:6}.atsumi-x-media-progress>span{display:block;height:100%;background:#1d9bf0}.atsumi-x-media-progress[data-atsumi-photo-progress]{position:fixed;right:auto;bottom:auto;z-index:2147483644}.atsumi-x-save-notice>.atsumi-x-media-progress{background:#ffffff40}.atsumi-x-save-notice>.atsumi-x-media-progress>span{background:#fff}.atsumi-x-media-progress[data-state="failed"]>span,.atsumi-x-media-progress[data-state="partial"]>span{background:#f4a340}.atsumi-x-media-progress[data-state="cancelled"]>span{background:#71767b}.atsumi-x-media-progress[data-state="complete"]>span{background:#00ba7c}.atsumi-x-media-progress[data-indeterminate]>span{animation:atsumi-x-save-progress 1.2s linear infinite}@keyframes atsumi-x-save-progress{from{transform:translateX(-100%)}to{transform:translateX(440%)}}@media(prefers-reduced-motion:reduce){.atsumi-x-media-progress[data-indeterminate]>span{animation:none}}';
  style.textContent+='.atsumi-x-save-notice button[hidden]{display:none!important}.atsumi-x-media-progress[data-state="unknown"]>span{background:#f4a340}';
  style.textContent+='.atsumi-x-save-notice{width:max-content;max-width:calc(100vw - 32px);max-height:min(240px,35vh);overflow:auto;padding:0;background:#000;color:#e7e9ea;border:1px solid #2f3336;box-sizing:border-box}.atsumi-x-save-row{position:relative;padding:9px 12px;overflow-wrap:anywhere}.atsumi-x-save-row+.atsumi-x-save-row{border-top:1px solid #2f3336}.atsumi-x-save-row>.atsumi-x-media-progress{background:#1d9bf033}.atsumi-x-save-row button:focus-visible{outline:2px solid #1d9bf0;outline-offset:2px}.atsumi-x-save-row [data-atsumi-save-busy]{font-size:12px;color:#8b98a5}';
  style.textContent+='[data-atsumi-post-save]{color:#71767b!important;align-self:center!important;margin-block:0!important;box-sizing:border-box!important}.atsumi-x-download-rail{position:absolute;top:8px;right:8px;z-index:5;display:flex;align-items:center;justify-content:flex-end;gap:4px;width:32px;box-sizing:content-box;opacity:0;pointer-events:none;transition:width .12s ease,opacity .12s ease}.atsumi-x-media-host:hover>.atsumi-x-download-rail,.atsumi-x-media-host:focus-within>.atsumi-x-download-rail{opacity:1;pointer-events:auto}.atsumi-x-download-rail[data-expanded]{width:68px}.atsumi-x-download-rail>.atsumi-x-inline-action{background:#000b!important}.atsumi-x-download-rail>[hidden]{display:none!important}@media(prefers-reduced-motion:reduce){.atsumi-x-download-rail{transition:none}}';
  style.textContent+='.atsumi-x-save-check{display:inline-block;width:24px;height:24px;vertical-align:middle;color:#00ba7c}.atsumi-x-save-check path{fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round;stroke-dasharray:1;stroke-dashoffset:0;animation:atsumi-x-save-check .45s ease-out both}@keyframes atsumi-x-save-check{from{stroke-dashoffset:1}to{stroke-dashoffset:0}}@media(prefers-reduced-motion:reduce){.atsumi-x-save-check path{animation:none;stroke-dashoffset:0}}';
  function installStyle() {
    // WebView2 runs the concatenated native helpers at document start, before
    // either <html> or <head> necessarily exists. Do not abort the following
    // input/surface initializers just because presentation cannot mount yet.
    const root=document.head||document.documentElement;
    if(!root)return false;
    if(!style.isConnected)root.append(style);
    return true;
  }
  if(!installStyle())document.addEventListener('DOMContentLoaded',installStyle,{once:true});
  const ownedControl = event => {
    if(!event.isTrusted||!(event.target instanceof Element))return null;
    const target=event.target.closest('button');
    return target?.isConnected&&controls.has(target)?target:null;
  };
  // Source-page ancestor-capture handlers can precede target-level onclick.
  // Isolate our controls before those handlers. Delegate only our exact
  // connected buttons here, before the native React root receives the gesture.
  for(const type of ['pointerdown','pointerup','mousedown','mouseup']){
    document.addEventListener(type,event=>{
      if(!ownedControl(event))return;
      if(type==='pointerdown'&&visible&&[0,2].includes(event.button)&&!event.ctrlKey&&!event.metaKey&&!event.altKey)signal('focus','');
      event.stopImmediatePropagation();
    },true);
  }
  for(const type of ['keydown','keyup']){
    document.addEventListener(type,event=>{
      if((event.key==='Enter'||event.key===' ')&&ownedControl(event)){
        if(type==='keydown'&&visible&&!event.ctrlKey&&!event.metaKey&&!event.altKey)signal('focus','');
        event.stopImmediatePropagation();
      }
    },true);
  }
  document.addEventListener('click',event=>{
    const target=ownedControl(event);if(!target)return;
    event.preventDefault();event.stopImmediatePropagation();
    if(visible)target.onclick?.call(target,event);
  },true);
  for(const type of ['pointerover','pointerout']){
    document.addEventListener(type,event=>{
      if(!event.isTrusted||!visible||!photoControl?.isConnected)return;
      const photo=currentPhoto(),target=type==='pointerout'?event.relatedTarget:event.target;
      const over=!!photo&&target instanceof Element&&(target===photo.image||target===photo.paint||photoControl.contains(target));
      photoControl.toggleAttribute('data-media-hover',over);
    },true);
  }
  document.addEventListener('contextmenu',e=>{
    if(!e.isTrusted||!visible||!(e.target instanceof Element)||e.target.closest('input,textarea,[contenteditable="true"]'))return;
    const link=e.target.closest('a[href]');
    if(link){try{const url=new URL(link.getAttribute('href'),location.href);if(!['x.com','www.x.com'].includes(url.hostname.replace(/\.$/,'')))return;}catch{return;}}
    const article=e.target.closest('article[data-testid="tweet"]');
    if(!article){
      const photo=currentPhoto();
      if(photo&&(e.target===photo.image||e.target===photo.paint)){
        e.preventDefault();e.stopImmediatePropagation();photoChoices(null,photo,{x:e.clientX,y:e.clientY});
      }
      return;
    }
    const path=pathFor(article);
    if(!path)return;
    if(!own(article,e.target)){
      if(!searchEnabled)return;
      // Quoted media can be searched without granting archive/download
      // ownership or inventing its canonical post/artist identity.
      const search=sourceSearchTarget(article,path,searchMediaRoot(article,e.target),e.target);if(!search)return;
      e.preventDefault();e.stopImmediatePropagation();window.__atsumiXSurface?.dismissMenu?.();closeMenu();
      menu=document.createElement('div');menu.className='atsumi-x-download-menu';menu.setAttribute('role','menu');menu.setAttribute('aria-label','미디어 검색');menu.append(searchItem(path,search));
      const href=link?.href,linked=href&&postPath(href);
      if(linked)menu.append(item('외부 브라우저에서 열기',()=>{if(search.current()&&link?.href===href)signal('external',new URL(href).pathname);}));
      placeMenu(null,{x:e.clientX,y:e.clientY});return;
    }
    const media=mediaRoot(article,e.target);
    // Leave profile, linked posts and quote targets to the exact-link context
    // handler. Media links still belong to the observed current post.
    if(e.target.closest('a[href]')&&!media)return;
    e.preventDefault();e.stopImmediatePropagation();
    choices(null,article,path,media,{x:e.clientX,y:e.clientY},e.target);
  },true);
  document.addEventListener('pointerdown',e=>{
    if(expansion&&!expansion.rail.contains(e.target))collapse();
    if(menu&&!menu.contains(e.target)&&!(e.target instanceof Element&&e.target.closest('[data-atsumi-media-save],[data-atsumi-photo-save]')))closeMenu();
  },true);
  document.addEventListener('keydown',e=>{if(e.isTrusted&&e.key==='Escape'&&(menu||expansion)){e.preventDefault();e.stopImmediatePropagation();closeMenu();}},true);
  window.__atsumiXActions=Object.freeze({
    update,
    photoDiagnostic,
    // Native-pane and main-shell focus coordination can dismiss UI without a
    // synthetic click or a focus change in the source document.
    dismissMenus:closeMenu,
    takeSearch(id){
      if(!searchEnabled)return null;
      const entry=searches.get(id),payload=entry?.payload;
      if(!entry||!payload)return null;
      entry.payload=null;clearTimeout(entry.expire);
      if(!searchAllowed(payload.kind)){clearSearch(id);return null;}
      try{if(!visible||Date.now()>entry.expires||location.href!==entry.sourceUrl||!entry.current()){clearSearch(id);return null;}}catch{clearSearch(id);return null;}
      return payload;
    },
    searchResult(id,value){
      if(!searchEnabled)return;
      const entry=searches.get(id);
      if(!entry||!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).length!==2
        ||!Object.hasOwn(value,'state')||!Object.hasOwn(value,'message')||!['opened','failed'].includes(value.state)||typeof value.message!=='string'||value.message.length>512)return;
      const current=visible&&location.href===entry.sourceUrl;clearSearch(id);
      if(current)searchNotice(value.message);
    },
    // The keyboard adapter may continue photo navigation from this one
    // app-owned control. No unrelated, stale or menu button gains that scope.
    ownsPhotoControl(node){
      return visible&&!menu&&!expansion&&node instanceof Element&&!!photoControl?.isConnected
        &&controls.has(photoControl)&&photoControl.contains(node)&&samePhoto(photoControlTarget,currentPhoto());
    },
    visibility(value){
      visible=value===true;
      if(!visible){for(const id of searches.keys())clearSearch(id);photoRefresh=null;closeMenu();toast?.remove();toast=null;notices.clear();photoControl?.remove();photoControl=null;photoControlTarget=null;for(const job of [...pending.values(),...finished.values()]){job.poll=null;clearTextTimer(job,true);clearLines(job);}}
      else{
        update();
        for(const job of pending.values()){paintJob(job);scheduleReceipt(job);}
      }
    },
    progress(id,value){
      const job=pending.get(id);if(!job||!acceptProgress(job,value))return;paintJob(job);
    },
    result:finishJob,
    receipt(id,value){
      const job=pending.get(id);
      if(!job||!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).length!==3
        ||!['state','progress','message'].every(key=>Object.hasOwn(value,key))||!receiptStates.has(value.state)
        ||typeof value.message!=='string'||value.message.length>512||value.progress!==null&&!progressValue(value.progress))return;
      const next=value.progress===null?null:progressValue(value.progress);
      if(next&&(next.sequence<job.lastSequence||next.sequence===job.lastSequence
        &&!sameProgress(next,job.progress)))return;
      if(value.state==='active'){
        // Accepted status can repeat, but an older empty acknowledgement or
        // contradictory terminal payload cannot regress newer native truth.
        if(job.terminal||next&&terminalStage(next.stage)||!next&&job.lastSequence>=0)return;
        // An authoritative still-active response may follow a lost cancel.
        // Permit only another explicit click, never resubmit it automatically.
        if(job.cancelling)job.cancelAwaiting=false;
        confirmed(job);if(next&&next.sequence>job.lastSequence)acceptProgress(job,next);paintJob(job);
      }else{
        const expected=value.state==='complete'||value.state==='partial'?'complete':value.state;
        if(value.state==='absent'?(next!==null||job.accepted)
          :next?next.stage!==expected||value.state==='partial'&&!next.partial
          :job.lastSequence>=0||value.state==='complete'||value.state==='partial')return;
        if(next){job.progress=next;job.lastSequence=next.sequence;}
        finishJob(id,value.state,value.message);
      }
    },
  });
})();
