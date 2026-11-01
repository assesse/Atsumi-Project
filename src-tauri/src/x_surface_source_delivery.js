// Public code compatibility only. No source player, React data or account access.
(() => {
  'use strict';
  const config = __ATSUMI_X_SOURCE_COMPAT__;
  if (location.protocol !== 'https:' || !['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com'].includes(location.hostname)
      || location.port || window.top !== window || !config.nonce) return;
  const key = 'webpackChunk_twitter_responsive_web', apiKey = '__atsumiXSourceCompat';
  if (Object.hasOwn(window, apiKey)) return;
  const before = Object.getOwnPropertyDescriptor(window, key);
  if (before && (!before.configurable || before.get || before.set)) return;
  let queue, currentPush, originalPush, pushGet, pushSet, closed = false;
  let delivery = null, depth = 0;
  const states = new Map((config.feed&&config.feed!=='preview'?['media','viewer','layout']:['media']).map(kind=>[kind,{delivered:false,originalSeen:false,closed:false}]));
  let errorClass = '';

  function target(packet) {
    const modules = Array.isArray(packet) && packet[1];
    if (!modules || typeof modules !== 'object') return null;
    const keys = Object.keys(modules);
    if (keys.length > 512) return null;
    let matches = [];
    for (const id of keys) {
      if (typeof modules[id] !== 'function') continue;
      const code = Function.prototype.toString.call(modules[id]);
      if (code.length <= 4 * 1024 * 1024 && code.includes('_renderBlurredMediaTombstone=()=>{')
          && code.includes('displaySensitiveMedia:') && code.includes('rweb_age_assurance_flow_enabled')) matches.push('media');
      if (states.has('viewer')&&code.length<=4*1024*1024&&code.includes('"explore_relaunch_enable_immersive_web","explore_relaunch_enable_video_carousels_immersive_viewer"')&&code.includes('"immersiveMediaViewer"'))matches.push('viewer');
      if (states.has('layout')&&code.length<=4*1024*1024&&code.includes('ImmersiveMediaViewerScreen:()=>')&&code.includes('testID:"modalContainer"'))matches.push('layout');
    }
    return matches.length===1?matches[0]:null;
  }
  function forward(fn, receiver, args) {
    depth++;
    try { return Reflect.apply(fn, receiver, args); }
    finally { depth--; }
  }
  function cleanup() {
    if (closed) return;
    closed = true;
    const descriptor = queue && Object.getOwnPropertyDescriptor(queue, 'push');
    if (descriptor?.get === pushGet && descriptor?.set === pushSet) {
      Object.defineProperty(queue, 'push', { configurable: true, writable: true, enumerable: false, value: originalPush });
    }
    const root = Object.getOwnPropertyDescriptor(window, key);
    if (root?.get === getQueue && root?.set === setQueue) {
      Object.defineProperty(window, key, { configurable: true, writable: true, enumerable: before?.enumerable ?? true, value: queue });
    }
    if (window[apiKey] === api) delete window[apiKey];
  }
  function release() {
    cleanup();
  }
  function finish(kind) { states.get(kind).closed=true;if([...states.values()].every(state=>state.closed))cleanup(); }
  function wrap(fn) {
    return function (...args) {
      if (closed || depth || args.length !== 1) return Reflect.apply(fn, this, args);
      const kind=target(args[0]),state=states.get(kind);
      if(!state)return Reflect.apply(fn,this,args);
      if(state.closed)return state.delivered?queue.length:Reflect.apply(fn,this,args);
      if (delivery) {
        if(delivery!==kind)throw new Error('Unexpected public source packet');
        const result = forward(fn, this, args);
        state.delivered = true;
        return result;
      }
      state.originalSeen = true;
      if (state.delivered) { finish(kind); return queue.length; }
      if(kind==='viewer'||kind==='layout'){
        const adapted=window.__atsumiXRouteAdapter?.adapt(config.nonce,args[0],kind);
        if(adapted){
          const result=forward(fn,this,[adapted]);state.delivered=true;finish(kind);return result;
        }
        if(kind==='layout')window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);
      }
      // Never hold a completed network script: webpack's onload would reject
      // its still-unregistered chunk before a later compilation could arrive.
      finish(kind);
      return Reflect.apply(fn, this, args);
    };
  }
  function getQueue() { return queue; }
  function setQueue(value) {
    if (closed || !Array.isArray(value) || queue && queue !== value) { release(); window[key] = value; return; }
    if (queue === value) return;
    queue = value;
    const descriptor = Object.getOwnPropertyDescriptor(queue, 'push');
    if (descriptor && (!descriptor.configurable || descriptor.get || descriptor.set)) { release(); return; }
    originalPush = queue.push;
    if (typeof originalPush !== 'function') { release(); return; }
    currentPush = wrap(originalPush);
    pushGet = () => currentPush;
    pushSet = fn => { originalPush = fn; currentPush = typeof fn === 'function' ? wrap(fn) : fn; };
    Object.defineProperty(queue, 'push', { configurable: true, enumerable: false, get: pushGet, set: pushSet });
  }
  const api = Object.freeze({
    deliver(nonce, compile, kind='media') {
      const state=states.get(kind);
      if (closed || !state || state.closed || state.delivered || nonce !== config.nonce || typeof compile !== 'function') return false;
      delivery = kind;
      try { compile(); }
      catch (error) { errorClass = ['Error','TypeError','SyntaxError','ReferenceError','EvalError','RangeError'].includes(error?.name) ? error.name : 'unknown'; finish(kind); }
      finally { delivery = null; }
      return state.delivered;
    },
    release(nonce) { if (nonce === config.nonce) release(); },
    status() { return { closed, kinds:Object.fromEntries(states), errorClass }; },
  });
  Object.defineProperty(window, apiKey, { configurable: true, value: api });
  Object.defineProperty(window, key, { configurable: true, enumerable: before?.enumerable ?? true, get: getQueue, set: setQueue });
  if (before?.value !== undefined) setQueue(before.value);
})();
