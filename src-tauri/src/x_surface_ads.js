// Cosmetic-only X promoted-placement filtering. No network, source node removal,
// account actions, private state, or advertiser/profile-name heuristics.
(() => {
  const config = __ATSUMI_X_ADS__;
  if (window !== window.top || !['https://x.com', 'https://www.x.com'].includes(location.origin)
    || window.__atsumiXAds) return;
  const articleSelector = 'article[data-testid="tweet"]', placementSelector = '[data-testid="placementTracking"]';
  const marker = 'data-atsumi-x-ad-hidden', markerValue = 'promoted-v1';
  const excluded = '[data-testid="tweetText"],[data-testid="tweetPhoto"],[data-testid="videoPlayer"],video,audio,iframe,[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)';
  const security = 'input[type="password"],input[autocomplete="one-time-code"],[data-testid="ocfEnterTextTextInput"],[data-testid="LoginForm_Login_Button"],a[href^="/i/flow/"],a[href="/login"]';
  const appNodes = '[data-atsumi-post-save],[data-atsumi-media-rail],[data-atsumi-photo-save],style[data-atsumi-x],style[data-atsumi-x-ads],.atsumi-x-download-menu,.atsumi-x-link-menu,.atsumi-x-save-notice';
  const marked = new Map();
  let enabled = false, observer = null, pending = null, style = null;
  const emptyHeader = () => ({ inspected:0, namesMissing:0, namesMultiple:0,
    stoppedContent:0, stoppedRoleLink:0, roleLinkInName:0, stoppedContext:0,
    caretMissing:0, caretMultiple:0, candidateLimit:0, depthLimit:0,
    labelAbsent:0, labelAuthor:0, labelControl:0, labelScope:0,
    labelNonLeaf:0, labelHidden:0, outerLabel:0 });
  const empty = () => ({ ready:false, articles:0, outerPlacements:0, headerBadges:0,
    promotedIndicators:0, hidden:0, ambiguous:0, limited:false,
    multiplePlacements:0, sharedPlacements:0, structureLimited:0, metadataMissing:0,
    headerShape:emptyHeader() });
  let last = empty();
  const own = (article, node) => node.closest(articleSelector) === article && !node.closest(excluded);
  function shown(node) {
    for (let current = node, depth = 0; current instanceof Element; current = current.parentElement, depth++) {
      if (depth >= 64) return false;
      const css = getComputedStyle(current);
      if (current.hidden || current.hasAttribute('inert') || current.getAttribute('aria-hidden') === 'true'
        || css.visibility === 'hidden' || css.visibility === 'collapse' || css.opacity === '0'
        || css.display === 'none' && !(marked.has(current) && current.getAttribute(marker) === markerValue)) return false;
    }
    return node instanceof Element;
  }
  function primary() {
    if (!enabled || document.hidden || !/^[A-Za-z0-9_]{1,15}$/.test(config.owner || '')
      || !['https://x.com', 'https://www.x.com'].includes(location.origin)
      || /\/(?:photo|video)\/[1-4]$/.test(location.pathname)
      || location.pathname === '/login' || location.pathname.startsWith('/i/flow/')
      || document.querySelector(security) || document.fullscreenElement
      || document.querySelector('[role="dialog"],[aria-modal="true"]')) return null;
    const profiles = [...document.querySelectorAll('[data-testid="AppTabBar_Profile_Link"]')];
    const accounts = document.querySelectorAll('[data-testid="SideNav_AccountSwitcher_Button"]');
    const columns = document.querySelectorAll('[data-testid="primaryColumn"]');
    if (!profiles.length || profiles.length > 8 || accounts.length !== 1 || columns.length !== 1) return null;
    for (const profile of profiles) {
      try {
        const url = new URL(profile.getAttribute('href'), location.origin);
        const handle = /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(url.pathname)?.[1]?.toLowerCase();
        if (!['https://x.com', 'https://www.x.com'].includes(url.origin) || url.username || url.password
          || url.search || url.hash || handle !== config.owner.toLowerCase()) return null;
      } catch { return null; }
    }
    return shown(columns[0]) ? columns[0] : null;
  }
  function classify(article, column) {
    const result = { target:null, placement:false, badge:false, indicator:false, ambiguous:false,
      multiplePlacements:false, sharedPlacements:false, structureLimited:false, metadataMissing:false };
    if (!article?.isConnected || !column.contains(article) || article.parentElement?.closest(articleSelector)
      || article.closest('[role="dialog"],[aria-modal="true"]') || !shown(article)) return result;
    const nodes = article.querySelectorAll('*');
    if (nodes.length > 1024) { result.ambiguous = true; result.structureLimited = true; return result; }
    const indicators = [...article.querySelectorAll('[data-testid="promotedIndicator"]')]
      .filter(node => own(article, node) && !node.closest('[data-testid="User-Name"],[data-testid="socialContext"]') && shown(node));
    if (indicators.length > 1) { result.ambiguous = true; return result; }
    result.indicator = indicators.length === 1;
    const placements = new Set();
    let ancestor = article.parentElement, depth = 0;
    for (; ancestor && ancestor !== column && depth < 32; ancestor = ancestor.parentElement, depth++) {
      if (ancestor.matches(placementSelector)) placements.add(ancestor);
    }
    const cell = article.closest('[data-testid="cellInnerDiv"]');
    if (cell && column.contains(cell)) {
      const tracking = [...cell.querySelectorAll(placementSelector)];
      if (tracking.length > 16) { result.ambiguous = true; result.structureLimited = true; return result; }
      // Source placements may be a sibling of the article in its cell, not
      // an ancestor. Inner video tracking is always excluded.
      for (const node of tracking) if (!node.closest(articleSelector)) placements.add(node);
    }
    if (ancestor !== column || placements.size > 16) {
      result.ambiguous = true;
      result.structureLimited = true;
      return result;
    }
    result.multiplePlacements = placements.size > 1;
    let placementTarget = null;
    if (result.multiplePlacements) {
      const posts = cell?.querySelectorAll(articleSelector);
      result.sharedPlacements = !!posts && (posts.length !== 1 || posts[0] !== article);
      // Hydrated X ads can have several outer tracking markers. All must
      // belong to this same exclusive source cell. Do not merge scopes or
      // infer ownership from matching text, URLs, order or a nearby tweet.
      if (!cell || !column.contains(cell) || !posts || posts.length !== 1 || posts[0] !== article
        || !shown(cell) || [...placements].some(node => !node.isConnected || !cell.contains(node)
          || node.closest('[data-testid="cellInnerDiv"]') !== cell || node.closest(articleSelector))) {
        result.ambiguous = true; return result;
      }
      placementTarget = cell;
      result.placement = true;
    } else if (placements.size === 1) {
      const placement = [...placements][0];
      placementTarget = placement.contains(article) ? placement : cell;
      const posts = placementTarget?.querySelectorAll(articleSelector);
      if (!posts || posts.length !== 1 || posts[0] !== article || !shown(placementTarget)) {
        result.ambiguous = true;
        result.sharedPlacements = !posts || posts.length !== 1 || posts[0] !== article;
        return result;
      }
      result.placement = true;
    }
    const names = [...article.querySelectorAll('[data-testid="User-Name"]')].filter(node => own(article, node));
    if (names.length === 1) {
      let header = names[0].parentElement, distance = 0;
      for (; header && header !== article && distance < 8; header = header.parentElement, distance++) {
        // A header must stop before body/media/context. The word "Ad" in any
        // of those, or inside an author name, is never promotion evidence.
        if (header.querySelector(excluded + ',[data-testid="socialContext"]')) break;
        const carets = [...header.querySelectorAll('[data-testid="caret"]')].filter(node => own(article, node));
        if (carets.length !== 1) continue;
        const badges = [...header.querySelectorAll('span,div')];
        if (badges.length > 64) break;
        result.badge = badges.some(node => own(article, node) && !node.closest('[data-testid="User-Name"],a,button,[role="button"]')
          && node.childElementCount === 0 && shown(node) && /^(?:Ad|Promoted|광고)$/i.test((node.textContent || '').trim()));
        break;
      }
    }
    result.metadataMissing = result.placement && !result.badge && (result.multiplePlacements || !result.indicator);
    // Multiple markers require the independent own header badge even if an
    // own promotedIndicator also exists. Single-marker behavior is unchanged.
    if (result.multiplePlacements ? result.placement && result.badge : result.indicator || result.placement && result.badge) {
      result.target = placementTarget || article;
    }
    return result;
  }
  // Debug-only explanation of an already rejected, exclusively owned placement.
  // Never feeds classification, changes source nodes, or returns source strings.
  function headerDiagnostic(article) {
    const result = emptyHeader(); result.inspected = 1;
    const names = [...article.querySelectorAll('[data-testid="User-Name"]')].filter(node => own(article,node));
    if (names.length !== 1) {
      result.namesMissing = Number(!names.length); result.namesMultiple = Number(names.length > 1); return result;
    }
    let header = names[0].parentElement, distance = 0, firstCaret = false;
    for (; header && header !== article && distance < 8; header = header.parentElement, distance++) {
      const content = header.querySelector('[data-testid="tweetText"],[data-testid="tweetPhoto"],[data-testid="videoPlayer"],video,audio,iframe,[data-testid="quoteTweet"],[data-testid="quotedTweet"]');
      const roleLinks = [...header.querySelectorAll('[role="link"]:not(a)')];
      const context = header.querySelector('[data-testid="socialContext"]');
      if (content || roleLinks.length || context) {
        // A stop after the first eligible header is only an outer-search boundary.
        if (!firstCaret) {
          result.stoppedContent = Number(!!content); result.stoppedRoleLink = Number(!!roleLinks.length);
          result.roleLinkInName = Number(roleLinks.some(node => names[0].contains(node)));
          result.stoppedContext = Number(!!context);
        }
        break;
      }
      const carets = [...header.querySelectorAll('[data-testid="caret"]')].filter(node => own(article,node));
      if (carets.length !== 1) {
        if (!firstCaret) { result.caretMissing = Number(!carets.length); result.caretMultiple = Number(carets.length > 1); }
        continue;
      }
      const badges = [...header.querySelectorAll('span,div')];
      if (badges.length > 64) { if (!firstCaret) result.candidateLimit = 1; break; }
      const labels = badges.filter(node => /^(?:Ad|Promoted|광고)$/i.test((node.textContent || '').trim()));
      const eligible = node => own(article,node) && !node.closest('[data-testid="User-Name"],a,button,[role="button"]')
        && node.childElementCount === 0 && shown(node);
      if (firstCaret) { if (labels.some(eligible)) { result.outerLabel = 1; break; } continue; }
      firstCaret = true; result.labelAbsent = Number(!labels.length);
      for (const node of labels) {
        result.labelAuthor |= Number(!!node.closest('[data-testid="User-Name"]'));
        result.labelControl |= Number(!!node.closest('a,button,[role="button"]'));
        result.labelScope |= Number(!own(article,node));
        result.labelNonLeaf |= Number(node.childElementCount !== 0);
        result.labelHidden |= Number(!shown(node));
      }
    }
    result.depthLimit = Number(distance >= 8 && header !== article);
    return result;
  }
  function restore(target) {
    const record = marked.get(target);
    if (!record) return;
    if (target.getAttribute(marker) === markerValue) {
      if (record.previous === null) target.removeAttribute(marker);
      else target.setAttribute(marker, record.previous);
    }
    marked.delete(target);
  }
  function mark(target, article) {
    if (!marked.has(target)) marked.set(target, { article, previous:target.getAttribute(marker) });
    else marked.get(target).article = article;
    if (target.getAttribute(marker) !== markerValue) target.setAttribute(marker, markerValue);
    // Already-playing native media must not continue audibly behind a cosmetic
    // hidden row. Do not touch iframe contents, src, volume, mute or playback of
    // any ordinary row; the surface play guard handles later race-time events.
    for (const media of [...article.querySelectorAll('video,audio')].slice(0,16)) {
      if (!media.paused) { try { media.pause(); } catch {} }
    }
  }
  function attachStyle() {
    const root = document.head || document.documentElement;
    if (!root) return false;
    if (!style) {
      style = document.createElement('style'); style.setAttribute('data-atsumi-x-ads', 'true');
      style.textContent = `[${marker}="${markerValue}"]{display:none!important}`;
    }
    if (!style.isConnected) root.append(style);
    return true;
  }
  function reconcile() {
    pending = null;
    const column = primary(), next = empty();
    if (!column || !attachStyle()) {
      for (const target of [...marked.keys()]) restore(target);
      last = next; return;
    }
    next.ready = true;
    const articles = [...column.querySelectorAll(articleSelector)];
    next.articles = Math.min(articles.length,160);
    if (articles.length > 160) {
      next.limited = true;
      for (const target of [...marked.keys()]) restore(target);
      last = next; return;
    }
    const wanted = new Map();
    for (const article of articles) {
      const proof = classify(article,column);
      next.outerPlacements += Number(proof.placement); next.headerBadges += Number(proof.badge);
      next.promotedIndicators += Number(proof.indicator); next.ambiguous += Number(proof.ambiguous);
      next.multiplePlacements += Number(proof.multiplePlacements);
      next.sharedPlacements += Number(proof.sharedPlacements);
      next.structureLimited += Number(proof.structureLimited);
      next.metadataMissing += Number(proof.metadataMissing);
      if (config.diagnostics === true && proof.metadataMissing) {
        const header = headerDiagnostic(article);
        for (const key of Object.keys(next.headerShape)) next.headerShape[key] += header[key];
      }
      if (proof.target) wanted.set(proof.target,article);
    }
    for (const target of [...marked.keys()]) if (!wanted.has(target)) restore(target);
    for (const [target,article] of wanted) mark(target,article);
    next.hidden = wanted.size; last = next;
  }
  const isApp = node => (node instanceof Element ? node : node.parentElement)?.closest(appNodes);
  function relevant(record) {
    if (isApp(record.target)) return false;
    if (record.type === 'attributes') return record.oldValue !== record.target.getAttribute(record.attributeName);
    if (record.type === 'childList') {
      const nodes = [...record.addedNodes,...record.removedNodes];
      return !nodes.length || nodes.some(node => !isApp(node));
    }
    return true;
  }
  function schedule(records) {
    if (!enabled || pending !== null || !records.some(relevant)) return;
    const token = {}; pending = token;
    // One bounded pass for a burst, not one full timeline scan per video
    // progress/style tick. Race-time media/actions still call blocked directly.
    setTimeout(() => { if (pending === token && enabled) reconcile(); }, 100);
  }
  function clear() {
    enabled = false; pending = null; observer?.disconnect(); observer = null;
    for (const target of [...marked.keys()]) restore(target);
    style?.remove(); last = empty();
  }
  window.__atsumiXAds = Object.freeze({
    update(ready) {
      if (ready !== true) { clear(); return; }
      enabled = true; reconcile();
      if (!observer && document.documentElement) {
        observer = new MutationObserver(schedule);
        observer.observe(document.documentElement, { subtree:true, childList:true, characterData:true,
          attributes:true, attributeOldValue:true,
          attributeFilter:['data-testid','href','aria-label','role','hidden','aria-hidden','inert','type','autocomplete','class','style'] });
      }
    },
    blocked(node) {
      if (!(node instanceof Element) || !node.isConnected) return false;
      const column = primary();
      if (!column) { for (const target of [...marked.keys()]) restore(target); return false; }
      const article = node.closest(articleSelector);
      const proof = classify(article,column);
      for (const [target,record] of [...marked]) {
        if (!target.isConnected || (record.article === article || target.contains(node)) && target !== proof.target) restore(target);
      }
      if (!proof.target) return false;
      if (attachStyle()) mark(proof.target,article);
      return proof.target.contains(node);
    },
    clear,
    diagnostic() { return config.diagnostics === true ? { ...last, headerShape:{ ...last.headerShape } } : null; },
  });
})();
