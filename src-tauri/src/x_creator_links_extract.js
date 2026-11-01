// Read only the expected profile's rendered header. Never inspect tweet media,
// application state, credentials, or inferred identities from search images.
(() => {
  const request = __ATSUMI_X_CREATOR_REQUEST__;
  const out = (state, extra = {}) => ({ nonce: request.nonce, state, ...extra });
  const clean = value => String(value || '').replace(/\s+/g, ' ').trim();
  const boundedLabel = value => {
    let result = '';
    for (const character of value) {
      if (result.length + character.length > 128) break;
      result += character;
    }
    return result;
  };
  const handle = /^[A-Za-z0-9_]{1,15}$/.test(request.handle || '') ? request.handle.toLowerCase() : null;
  if (!handle || window !== window.top || location.protocol !== 'https:' ||
      !['x.com', 'www.x.com'].includes(location.hostname) || location.port ||
      location.pathname.toLowerCase().replace(/\/$/, '') !== '/' + handle) return out('unavailable');
  const excluded = node => !!node.closest('article,[role="feed"],aside,[role="complementary"],[role="dialog"],[data-testid="UserCell"]');
  const visible = node => {
    for (let current = node; current && current !== document.body; current = current.parentElement) {
      // The transport accessibility guard intentionally marks direct body roots
      // inert/aria-hidden. Descendant hidden UI still must not supply evidence.
      if (current.hidden || (current.parentElement !== document.body &&
          (current.getAttribute('aria-hidden') === 'true' || current.hasAttribute('inert')))) return false;
      const style = window.getComputedStyle(current);
      if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || style.opacity === '0') return false;
    }
    return true;
  };
  const text = node => {
    let result = '';
    for (const child of node.childNodes) {
      if (child.nodeType === 3) result += child.textContent || '';
      else if (child.nodeType === 1 && visible(child)) result += ' ' + text(child);
    }
    return clean(result);
  };
  let owner = null;
  let accountVisible = false;
  if (request.owner) {
    const profile = document.querySelector('[data-testid="AppTabBar_Profile_Link"]');
    try {
      const url = new URL(profile?.getAttribute('href') || '', 'https://x.com');
      if (url.origin === 'https://x.com') owner = /^\/([A-Za-z0-9_]{1,15})\/?$/.exec(url.pathname)?.[1]?.toLowerCase();
    } catch {}
    accountVisible = !!document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
    if (owner && owner !== request.owner.toLowerCase()) return out('ownerChanged');
  }
  const columns = [...document.querySelectorAll('[data-testid="primaryColumn"]')].filter(visible);
  let groups;
  if (columns.length === 1) {
    // Existing authenticated app profile: retain its owner and test-ID gates.
    if (request.owner && (!owner || !accountVisible)) return out('pending');
    const primary = columns[0];
    const names = [...primary.querySelectorAll('[data-testid="UserName"]')].filter(node => visible(node) && !excluded(node));
    if (names.length !== 1) return out('pending');
    const observedHandles = [...text(names[0]).matchAll(/(?:^|\s)@([A-Za-z0-9_]{1,15})(?=$|\s)/g)].map(match => match[1].toLowerCase());
    if (observedHandles.length !== 1 || observedHandles[0] !== handle) return out('unavailable');
    const firstPost = primary.querySelector('article,[role="feed"]');
    const beforePosts = node => !firstPost || !!(node.compareDocumentPosition(firstPost) & 4);
    groups = [...primary.querySelectorAll('[data-testid="UserDescription"],[data-testid="UserUrl"]')]
      .filter(node => visible(node) && !excluded(node) && beforePosts(node));
    if (groups.filter(node => node.getAttribute('data-testid') === 'UserDescription').length > 1 ||
        groups.filter(node => node.getAttribute('data-testid') === 'UserUrl').length > 1) return out('unavailable');
  } else {
    // The official public static profile observed 2026-10-24 has no React
    // test IDs. Admit only its profile header, never arbitrary page anchors.
    if (columns.length) return out('pending');
    const exact = 'https://x.com/' + handle;
    const canonical = [...document.querySelectorAll('link[rel="canonical"]')];
    const openGraph = [...document.querySelectorAll('meta[property="og:url"]')];
    if (canonical.length !== 1 || canonical[0].getAttribute('href') !== exact ||
        openGraph.length !== 1 || openGraph[0].getAttribute('content') !== exact) return out('pending');
    const mains = [...document.querySelectorAll('main')].filter(node => visible(node) && !excluded(node));
    if (mains.length !== 1) return out('pending');
    const main = mains[0];
    const headings = [...main.querySelectorAll('h1')].filter(node => visible(node) && !excluded(node));
    if (headings.length !== 1 || !text(headings[0])) return out('pending');
    const heading = headings[0];
    let header = heading.parentElement;
    let recognizedHeader = false;
    for (let depth = 0; header && depth < 6; depth++, header = header.parentElement) {
      if (header.classList.contains('@container') && [...header.classList].some(token => token.startsWith('[--profile-avatar-size:'))) { recognizedHeader = true; break; }
    }
    if (!recognizedHeader || !header || header === main || header.parentElement?.parentElement !== main || excluded(header) ||
        header.querySelector('article,[role="feed"],aside,[role="complementary"],[role="dialog"],[data-testid="UserCell"]')) return out('unavailable');
    const firstPost = main.querySelector('article,[role="feed"]');
    if (firstPost && !(header.compareDocumentPosition(firstPost) & 4)) return out('unavailable');
    const identity = heading.parentElement?.parentElement;
    if (!identity || identity.tagName !== 'DIV' || identity.parentElement !== header.firstElementChild ||
        identity.children.length !== 2 || identity.children[0].tagName !== 'SPAN' || identity.children[1].tagName !== 'SPAN' ||
        !identity.children[0].contains(heading) || !visible(identity.children[1]) ||
        text(identity.children[1]).toLowerCase() !== '@' + handle) return out('unavailable');
    const children = [...header.children].filter(node => visible(node) && !excluded(node));
    const bios = children.filter(node => node.tagName === 'DIV' && node.classList.contains('whitespace-pre-wrap') && node.classList.contains('text-body'));
    const websites = children.filter(node => node.tagName === 'DIV' && node.classList.contains('flex-wrap') && node.classList.contains('gap-x-3') && node.classList.contains('gap-y-1'));
    if (bios.length > 1 || websites.length > 1) return out('unavailable');
    groups = [...bios, ...websites];
  }
  const safeUrl = value => {
    if (typeof value !== 'string' || value.length > 2048 || /[\s\\\u0000-\u001f\u007f]/.test(value)) return null;
    try {
      const url = new URL(value);
      if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.port ||
          !url.hostname.includes('.') || url.hostname.endsWith('.') || /^\[|^[\d.]+$/.test(url.hostname) ||
          /(?:^|\.)(?:localhost|local|internal|test|invalid)$/.test(url.hostname)) return null;
      return url;
    } catch { return null; }
  };
  const links = [], seen = new Set();
  let unresolvedShortLinks = false;
  for (const group of groups) {
    const anchors = group.matches('a[href]') ? [group] : [...group.querySelectorAll('a[href]')];
    for (const anchor of anchors.slice(0, 20)) {
      if (!visible(anchor) || excluded(anchor)) continue;
      let url = safeUrl(anchor.getAttribute('href'));
      if (!url) continue;
      if (url.hostname === 't.co') {
        // Only full, explicitly rendered destination attributes are evidence.
        // Display ellipses, similar search candidates and guessed IDs are not.
        const expanded = ['data-expanded-url', 'title'].map(key => safeUrl(anchor.getAttribute(key)))
          .find(value => value && value.hostname !== 't.co');
        if (expanded) url = expanded;
        else unresolvedShortLinks = true;
      }
      if (['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com'].includes(url.hostname)) continue;
      if (seen.has(url.href)) continue;
      seen.add(url.href);
      links.push({ label: boundedLabel(text(anchor) || url.hostname), url: url.href });
      if (links.length === 8) break;
    }
    if (links.length === 8) break;
  }
  return out('ready', { profileUrl: 'https://x.com/' + handle, links, unresolvedShortLinks });
})()
