// Connect an entry offered by X's media renderer to the control on that same player.
(() => {
  'use strict';
  const config = __ATSUMI_X_SOURCE_COMPAT__;
  if (Object.hasOwn(window, '__atsumiXVideoEntry')) return;
  if (window !== window.top || location.protocol !== 'https:' || location.port ||
      !['x.com', 'www.x.com'].includes(location.hostname) || !config.nonce || config.feed === 'preview') return;

  const hosts = new WeakMap();
  const entries = new WeakMap();
  const hostSelector = '[data-atsumi-viewer-owner]';
  const registered = new Set();
  const prepared = new WeakMap();
  // Debug builds keep only the last 64 post IDs in memory to distinguish an
  // absent source URL from a lost binding when the fullscreen button is used.
  const observed = config.diagnostics === true ? new Map() : null;
  function observe(props, post) {
    if (!observed || !props || !Object.hasOwn(props, 'getMediaSessionMetadata') ||
        !Object.hasOwn(props, 'mediaPermalink') || !Object.hasOwn(props, 'authorScreenName')) return;
    const id = String(props.tweetId);
    if (!/^[1-9]\d{4,24}$/.test(id)) return;
    const bit = props.immersiveViewerUrl ? post?.id === id ? 2 : 4 : 1;
    const previous = observed.get(id) || 0;
    observed.delete(id);
    observed.set(id, previous | bit);
    if (observed.size > 64) observed.delete(observed.keys().next().value);
  }
  const ident = '[A-Za-z_$][\\w$]*';
  const escape = value => value.replace(/\$/g, '\\$');

  // The labels that survive minification are player properties. Read the two
  // control builders from their call sites, not from the enclosing chunk name.
  function playerShape(source) {
    if (source.length > 4 * 1024 * 1024) return null;
    const start = source.indexOf('_renderInlinePlayer=()=>{');
    if (start < 0 || !source.includes('_handleEnterImmersiveViewer=()=>{')) return null;
    const end = source.indexOf(';_playerState=', start);
    if (end < 0 || end - start > 8192) return null;
    const method = source.slice(start, end);
    if (!method.includes('immersiveViewerUrl:') || !method.includes('onPlayerState:this._onPlayerState')) return null;
    const controls = new RegExp('\\?\\(0,(' + ident + ')\\.(' + ident + ')\\)\\((' + ident + '),this\\._handleEnterImmersiveViewer\\):this\\.props\\.uiStack\\|\\|\\(0,\\1\\.(' + ident + ')\\)\\(\\3,this\\.props\\.onClickBackground\\)').exec(method);
    if (!controls || !method.includes('showControls:!1,uiStack:') ||
        !source.includes('history.push(this.props.immersiveViewerUrl||this.context.history.location.pathname)')) return null;
    const builders = importId(source, controls[1]);
    const call = new RegExp('\\(0,(' + ident + ')\\.jsx\\)\\(' + ident + '\\.' + ident + ',\\{\\.\\.\\.' + ident + ',showControls:!1,uiStack:', 'g');
    const runtimes = new Set([...method.matchAll(call)].map(match => match[1]));
    const runtime = runtimes.size === 1 ? importId(source, [...runtimes][0]) : null;
    return builders ? {builders, runtime, immersive: controls[2], desktop: controls[4]} : null;
  }

  function importId(source, alias) {
    const header = new RegExp('^(?:function(?: ' + ident + ')?|' + ident + '|[0-9]+)?\\(' + ident + ',' + ident + ',(' + ident + ')\\)\\{').exec(source.trim());
    if (!header) return null;
    const pattern = new RegExp('(?:^|[;,{}])(?:var |let |const )?' + escape(alias) + '=' + escape(header[1]) + '\\(([1-9][0-9]{0,8})\\)', 'g');
    const matches = [...source.matchAll(pattern)];
    return matches.length === 1 ? Number(matches[0][1]) : null;
  }

  function publisherShape(source) {
    if (source.length > 4 * 1024 * 1024 || !source.includes('tweetMediaItemImmersiveViewerEntryPoint:') ||
        !source.includes('/mediaviewer') || !source.includes('authorScreenName:') || !source.includes('tweetId:')) return null;
    const pattern = new RegExp('\\(0,(' + ident + ')\\.jsx\\)\\((' + ident + '\\.' + ident + '),\\{\\.\\.\\.(' + ident + '),immersiveViewerUrl:' + ident + '\\}\\):\\(0,\\1\\.jsx\\)\\(\\2,\\{\\.\\.\\.\\3\\}\\)', 'g');
    const matches = [...source.matchAll(pattern)];
    return matches.length === 1 ? importId(source, matches[0][1]) : null;
  }

  function entryShape(source) {
    if (source.length > 4 * 1024 * 1024 || source.split('_isClientAllowedIntoImmersiveViewer=').length !== 2 ||
        !source.includes('tweetMediaItemImmersiveViewerEntryPoint:') ||
        !source.includes('explore_relaunch_enable_immersive_web')) return null;
    const check = new RegExp('!\\((' + ident + ')\\.A\\.get\\("screen"\\)\\.width>=(' + ident + ')\\.A\\.theme\\.breakpoints\\.large\\)').exec(source);
    if (!check || source.split(check[1] + '.A.get("screen")').length !== 2) return null;
    const dimensions = importId(source, check[1]), theme = importId(source, check[2]);
    return dimensions && theme && dimensions !== theme ? {dimensions, theme} : null;
  }

  function wrapEntry(original, shape) {
    let largeBreakpoint = null;
    return scopedModule(original, (id, mod) => {
      if (id === shape.theme) {
        const large = mod?.A?.theme?.breakpoints?.large;
        if (typeof large === 'number' && Number.isFinite(large) && large > 1) largeBreakpoint = large;
        return mod;
      }
      if (id !== shape.dimensions) return mod;
      const dimensions = mod?.A;
      if (!dimensions || typeof dimensions.get !== 'function') return mod;
      const scoped = Object.create(mod), viewport = Object.create(dimensions);
      Object.defineProperty(viewport, 'get', {value: function(measure, ...rest) {
        if (measure === 'screen' && largeBreakpoint !== null &&
            window.__atsumiXSurface?.sourceMediaViewerAvailable?.() === true) {
          const current = Reflect.apply(dimensions.get, dimensions, ['window', ...rest]);
          if (current && typeof current.width === 'number' && Number.isFinite(current.width)) {
            return {...current, width: Math.min(current.width, largeBreakpoint - 1)};
          }
        }
        return Reflect.apply(dimensions.get, dimensions, [measure, ...rest]);
      }});
      Object.defineProperty(scoped, 'A', {value: viewport});
      return scoped;
    });
  }

  function scopedModule(original, decorate) {
    return function(exports, output, require) {
      if (typeof require !== 'function') return Reflect.apply(original, this, arguments);
      const scoped = new Proxy(require, {apply(target, receiver, args) {
        return decorate(args[0], Reflect.apply(target, receiver, args));
      }});
      return Reflect.apply(original, this, [exports, output, scoped]);
    };
  }

  function sourcePost(href) {
    if (typeof href !== 'string' || !href || href.length > 2048) return null;
    let url;
    try { url = new URL(href, location.href); } catch { return null; }
    const path = /^(\/(?:[a-z0-9_]{1,15}|i\/web)\/status\/([1-9]\d{4,24}))\/mediaviewer\/?$/i.exec(url.pathname);
    if (!path || url.origin !== new URL(location.href).origin || url.username || url.password || url.hash ||
        url.searchParams.getAll('currentTweet').some(id => id !== path[2])) return null;
    return {path: path[1], id: path[2]};
  }

  function resolvedViewerHref(props, mediaIndex, listLength) {
    if (props?.immersiveViewerUrl) return props.immersiveViewerUrl;
    if (window.__atsumiXSurface?.sourceMediaViewerAvailable?.() !== true) return null;
    if (!props || props.promotedContent || props.type === 'animated_gif' ||
        !Object.hasOwn(props, 'getMediaSessionMetadata') || !Object.hasOwn(props, 'mediaPermalink')) return null;
    const author = props.authorScreenName, id = String(props.tweetId ?? '');
    if (typeof author !== 'string' || !/^[A-Za-z0-9_]{1,15}$/.test(author) || !/^[1-9]\d{4,24}$/.test(id)) return null;
    const onProfile = location.pathname.toLowerCase() === '/' + author.toLowerCase();
    const params = [];
    if (onProfile) params.push('mode=profile');
    if (listLength > 1 && mediaIndex >= 1) params.push('mediaIndex=' + mediaIndex);
    return '/' + author + '/status/' + id + '/mediaviewer' + (params.length ? '?' + params.join('&') : '');
  }

  function mediaToken(props, fromList, resolvedHref) {
    const href = resolvedHref ?? props?.immersiveViewerUrl;
    const post = sourcePost(href);
    if (observed) observe(href === props?.immersiveViewerUrl ? props : {...props, immersiveViewerUrl: href}, post);
    if (!post || post.id !== String(props?.tweetId)) return null;
    const token = {};
    entries.set(token, {post, href, fromList, open: null});
    return token;
  }

  function committedElement(jsx, key, child, entry) {
    let previous;
    return jsx('div', {
      'data-atsumi-viewer-owner': '', style: {display: 'contents'},
      ref(node) {
        if (previous) hosts.delete(previous);
        previous = node;
        if (node) hosts.set(node, entry);
      },
      children: child,
    }, key);
  }

  function wrapPublisher(original, runtime) {
    return scopedModule(original, (id, mod) => {
      if (id !== runtime || typeof mod?.jsx !== 'function') return mod;
      const jsx = mod.jsx;
      return Object.defineProperty(Object.create(mod), 'jsx', {value: function(component, props, key) {
        if (typeof component !== 'string' && Array.isArray(props?.media) && props.media.length <= 4 &&
            Object.hasOwn(props, 'renderVideo') && Object.hasOwn(props, 'withCarouselPageIndicator')) {
          let tagged;
          for (const [index, item] of props.media.entries()) {
            if (item?.type !== 'video') continue;
            const href = resolvedViewerHref(item, index + 1, props.media.length);
            const token = mediaToken(item, true, href);
            if (!token) continue;
            tagged ||= props.media.slice();
            tagged[index] = href && !item.immersiveViewerUrl
              ? {...item, immersiveViewerUrl: href, atsumiViewerOwner: token}
              : {...item, atsumiViewerOwner: token};
          }
          return tagged ? jsx(component, {...props, media: tagged}, key) : Reflect.apply(jsx, mod, arguments);
        }
        const href = typeof component !== 'string' ? resolvedViewerHref(props, 0, 1) : null;
        const token = typeof component !== 'string' && mediaToken(props, false, href);
        if (!token) return Reflect.apply(jsx, mod, arguments);
        const nextProps = href && !props.immersiveViewerUrl
          ? {...props, immersiveViewerUrl: href, atsumiViewerOwner: token}
          : {...props, atsumiViewerOwner: token};
        const child = jsx(component, nextProps, key);
        return committedElement(jsx, key, child, entries.get(token));
      }});
    });
  }

  function wrapPlayer(original, shape) {
    const stacks = new WeakMap();
    return scopedModule(original, (id, mod) => {
      if (id === shape.runtime && typeof mod?.jsx === 'function') {
        const jsx = mod.jsx;
        return Object.defineProperty(Object.create(mod), 'jsx', {value: function(component, props, key) {
          const stack = props?.uiStack;
          const entry = props?.showControls === false && typeof props.onPlayerState === 'function' && stacks.get(stack);
          if (!entry) return Reflect.apply(jsx, mod, arguments);
          stacks.delete(stack);
          // Attach the ref at this particular list tile. The same video can
          // be rendered in two places, even from the same media configuration.
          return committedElement(jsx, key, Reflect.apply(jsx, mod, arguments), entry);
        }});
      }
      if (id !== shape.builders || typeof mod?.[shape.immersive] !== 'function' || typeof mod[shape.desktop] !== 'function') return mod;
      const build = mod[shape.immersive];
      return Object.defineProperty(Object.create(mod), shape.immersive, {value: function(props, open) {
        const entry = entries.get(props?.atsumiViewerOwner);
        if (!entry || entry.href !== props.immersiveViewerUrl || entry.post.id !== String(props.tweetId) ||
            typeof open !== 'function' || (entry.fromList && !shape.runtime)) return Reflect.apply(build, mod, arguments);
        if (!entry.fromList) {
          entry.open = open;
          return Reflect.apply(mod[shape.desktop], mod, arguments);
        }
        const stack = Reflect.apply(mod[shape.desktop], mod, arguments);
        if (!stack || (typeof stack !== 'object' && typeof stack !== 'function')) return Reflect.apply(build, mod, arguments);
        stacks.set(stack, {...entry, open});
        return stack;
      }});
    });
  }

  function prepare(packet) {
    if (registered.size === roles.length || !Array.isArray(packet) || !packet[1] || typeof packet[1] !== 'object') return packet;
    const modules = packet[1], ids = Object.keys(modules);
    if (ids.length > 512) return packet;
    const candidates = new Map();
    for (const id of ids) {
      const fn = modules[id];
      if (typeof fn !== 'function') continue;
      const code = Function.prototype.toString.call(fn);
      for (const [role, identify, wrap] of roles) {
        if (registered.has(role)) continue;
        const shape = identify(code);
        if (shape) candidates.set(role, candidates.has(role) ? null : {id, shape, wrap});
      }
    }
    const matches = [...candidates].filter(([, match]) => match);
    if (!matches.length) return packet;
    const copy = packet.slice();
    copy[1] = {...modules};
    for (const [, {id, shape, wrap}] of matches) copy[1][id] = wrap(copy[1][id], shape);
    prepared.set(copy, matches.map(([role]) => role));
    return copy;
  }
  const roles = [['address', publisherShape, wrapPublisher], ['player', playerShape, wrapPlayer], ['entry', entryShape, wrapEntry]];

  function open(event, root, control, postPath) {
    if (!event.isTrusted || !root?.isConnected || !root.contains(control) || typeof postPath !== 'string') return null;
    const enclosing = control.closest(hostSelector);
    const players = '[data-testid="videoPlayer"]';
    let host = enclosing;
    if (!host) {
      const inside = [...root.querySelectorAll(hostSelector)].filter(n => n.closest(players) === root);
      if (inside.length !== 1) return null;
      host = inside[0];
    }
    const owner = host.closest(players);
    if (owner ? owner !== root : !host.contains(root) || host.querySelectorAll(players).length !== 1) return null;
    const entry = hosts.get(host);
    if (!entry?.open || entry.post.path.toLowerCase() !== postPath.toLowerCase()) return null;
    const before = location.href;
    try { entry.open(); } catch { /* Preserve X's normal fullscreen action if its handler fails. */ }
    return location.href !== before;
  }
  function diagnosis(root, postPath) {
    if (!registered.has('address')) return registered.has('player') ? 'entry-wait-address' : 'entry-wait-both';
    if (!registered.has('player')) return 'entry-wait-player';
    const id = typeof postPath === 'string' && /\/status\/([1-9]\d{4,24})$/i.exec(postPath)?.[1];
    if (!id) return 'entry-no-post';
    const bits = observed.get(id);
    if (!bits) return 'entry-not-observed';
    if (bits === 1) return 'entry-no-url';
    if (bits === 4) return 'entry-unrecognized-url';
    if (bits !== 2) return 'entry-mixed';
    const host = root?.closest(hostSelector) || root?.querySelector(hostSelector);
    if (!host) return 'entry-no-host';
    return hosts.get(host)?.open ? 'entry-bound' : 'entry-no-handler';
  }
  Object.defineProperty(window, '__atsumiXVideoEntry', {value: Object.freeze({
    prepare, open, waiting: () => !registered.has('address') || !registered.has('player'),
    registered(packet) { for (const role of prepared.get(packet) || []) registered.add(role); },
    ...(observed ? {playerShape, publisherShape, entryShape, diagnosis} : {}),
  })});
})();
