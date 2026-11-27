// @ts-expect-error Node VM is used only by these isolated WebView tests.
import vm from 'node:vm';
import {describe, expect, it, vi} from 'vitest';
import bridge from '../../../src-tauri/src/x_surface_video_entry.js?raw';
import delivery from '../../../src-tauri/src/x_surface_source_delivery.js?raw';

// Structural fixtures of the two independently registered roles in X's renderer.
const publisher = 'function(m,exports,load){var j=load(11),C=load(17);const context={tweetMediaItemImmersiveViewerEntryPoint:()=>undefined};const suffix="/mediaviewer";exports.render=(id,url)=>{const props={authorScreenName:"alice",tweetId:id,mediaPermalink:undefined,getMediaSessionMetadata:()=>undefined};return url?(0,j.jsx)(C.A,{...props,immersiveViewerUrl:url}):(0,j.jsx)(C.A,{...props})};exports.list=media=>j.jsx(C.A,{media,renderVideo:undefined,withCarouselPageIndicator:false})}';
const player = 'function(m,exports,read){var helpers=read(29),j=read(11),Vid=read(17);class Player{_onPlayerState=()=>{};constructor(props,history){this.props=props;this.context={history}}_handleEnterImmersiveViewer=()=>{this.context.history.push(this.props.immersiveViewerUrl||this.context.history.location.pathname)};_renderInlinePlayer=()=>{let{immersiveViewerUrl:offer}=this.props,settings={...this.props},stack=offer?(0,helpers.compact)(settings,this._handleEnterImmersiveViewer):this.props.uiStack||(0,helpers.normal)(settings,this.props.onClickBackground);let base={onPlayerState:this._onPlayerState};return(0,j.jsx)(Vid.A,{...base,showControls:!1,uiStack:stack})};_playerState=null}exports.Player=Player}';
const href = '/alice/status/123456/mediaViewer?mode=profile';
const post = '/alice/status/123456';

function setup(order: string, feed = 'foryou', diagnostics = false) {
  const location = new URL('https://x.com/home');
  const runtime: Record<string, any> = {location, URL};
  runtime.window = runtime; runtime.top = runtime;
  const context = vm.createContext(runtime);
  for (const script of [bridge, delivery]) vm.runInContext(script.replace('__ATSUMI_X_SOURCE_COMPAT__', JSON.stringify({nonce:'test',feed,diagnostics})), context);
  const factory = (code: string) => vm.runInContext('(' + code + ')', context);
  const pub = {924: factory(publisher)}, play = {236: factory(player)};
  const queue: any[] = [];
  runtime.webpackChunk_twitter_responsive_web = queue;
  for (const modules of order === 'combined' ? [{...play, ...pub}] : order === 'player first' ? [play,pub] : [pub,play])
    runtime.webpackChunk_twitter_responsive_web.push([[113], modules]);
  const modules = Object.assign({}, ...queue.map((packet: any) => packet[1]));
  const builder = {compact: vi.fn(() => ({kind:'compact'})), normal: vi.fn(() => ({kind:'desktop'}))};
  const jsx = (component: unknown, props: Record<string, any>) => ({component, props});
  const imports: Record<number, any> = {11:{jsx},17:{A:class {}},29:builder};
  const address: Record<string, any> = {}, controls: Record<string, any> = {};
  modules[236]({}, controls, (id: number) => imports[id]);
  modules[924]({}, address, (id: number) => imports[id]);
  const mounted: HTMLElement[] = [];
  function mount(destination: string | undefined = href, id = '123456') {
    const element = address.render(id, destination);
    const host = document.createElement('div'), root = document.createElement('div'), button = document.createElement('button');
    root.dataset.testid = 'videoPlayer';
    host.setAttribute('data-atsumi-viewer-owner', '');
    root.append(button); host.append(root); document.body.append(host); mounted.push(host);
    const navigate = vi.fn((url: string) => { location.href = new URL(url, location).href; });
    const child = element.props.children || element;
    const instance = new controls.Player(child.props, {push:navigate,location});
    const stack = instance._renderInlinePlayer().props.uiStack.kind;
    element.props.ref?.(host);
    return {host,root,button,stack,navigate,unmount:()=>element.props.ref?.(null)};
  }
  function list(items: Array<{id:string, url?:string, type?:string}>) {
    return address.list(items.map(({id,url,type = 'video'}) => ({
      tweetId:id, immersiveViewerUrl:url, type, authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined,
    }))).props.media as Record<string, any>[];
  }
  function mountTile(props: Record<string, any>) {
    const root = document.createElement('div'), host = document.createElement('div'), button = document.createElement('button');
    root.dataset.testid = 'videoPlayer'; root.append(button); host.append(root); document.body.append(host); mounted.push(host);
    const navigate = vi.fn((url: string) => { location.href = new URL(url, location).href; });
    const rendered = new controls.Player({...props}, {push:navigate,location})._renderInlinePlayer();
    if (rendered.props.ref) {
      host.setAttribute('data-atsumi-viewer-owner', '');
      rendered.props.ref(host);
    }
    return {root,button,navigate,unmount:()=>rendered.props.ref?.(null)};
  }
  return {api:runtime.__atsumiXVideoEntry, location, mount, list, mountTile, builder, cleanup:()=>mounted.forEach(n=>n.remove())};
}

describe('source-owned X viewer actions', () => {
  it.each(['combined','address first','player first'])('uses the exact offered destination with %s registration', order => {
    const a = setup(order);
    try {
      const item = a.mount();
      expect(a.api.waiting()).toBe(false);
      expect(item.stack).toBe('desktop');
      expect(a.api.open({isTrusted:true},item.root,item.button,post)).toBe(true);
      expect(item.navigate).toHaveBeenCalledExactlyOnceWith(href);
      expect(a.location.search).toBe('?mode=profile');
    } finally { a.cleanup(); }
  });

  it.each(['','/bob/status/654321/mediaViewer','https://example.org/alice/status/123456/mediaViewer',
    '/alice/status/123456/video/1','/alice/status/123456/mediaViewer?currentTweet=654321'])('keeps the source outcome for an unavailable or unrelated destination (%s)', value => {
    const a = setup('combined');
    try {
      const item = a.mount(value);
      expect(a.api.open({isTrusted:true},item.root,item.button,post)).toBeNull();
      expect(item.navigate).not.toHaveBeenCalled();
      expect(item.stack).toBe(value ? 'compact' : 'desktop');
    } finally { a.cleanup(); }
  });

  it('keeps two instances of a post separate and releases a recycled instance', () => {
    const a = setup('player first');
    try {
      const first = a.mount(), second = a.mount();
      expect(a.api.open({isTrusted:true},first.root,second.button,post)).toBeNull();
      expect(a.api.open({isTrusted:false},first.root,first.button,post)).toBeNull();
      expect(a.api.open({isTrusted:true},first.root,first.button,'/alice/status/654321')).toBeNull();
      first.unmount();
      expect(a.api.open({isTrusted:true},first.root,first.button,post)).toBeNull();
      expect(a.api.open({isTrusted:true},second.root,second.button,post)).toBe(true);
      expect(first.navigate).not.toHaveBeenCalled();
      expect(second.navigate).toHaveBeenCalledOnce();
    } finally { a.cleanup(); }
  });

  it('binds each offered gallery video even when two tiles reuse the same media configuration', () => {
    const a = setup('player first', 'foryou', true);
    try {
      const second = post + '/mediaViewer?mediaIndex=2';
      const media = a.list([{id:'123456',url:href},{id:'123456',url:second},{id:'654321'},{id:'876543',type:'photo'}]);
      const first = a.mountTile(media[0]!), copy = a.mountTile(media[0]!), next = a.mountTile(media[1]!);
      const missing = a.mountTile(media[2]!);
      expect(a.api.diagnosis(missing.root, '/alice/status/654321')).toBe('entry-no-url');
      expect(a.api.diagnosis(missing.root, '/alice/status/876543')).toBe('entry-not-observed');
      first.unmount();
      expect(a.api.open({isTrusted:true},first.root,first.button,post)).toBeNull();
      expect(a.api.open({isTrusted:true},next.root,copy.button,post)).toBeNull();
      expect(a.api.open({isTrusted:true},copy.root,copy.button,post)).toBe(true);
      expect(copy.navigate).toHaveBeenCalledExactlyOnceWith(href);
      expect(a.api.open({isTrusted:true},next.root,next.button,post)).toBe(true);
      expect(next.navigate).toHaveBeenCalledExactlyOnceWith(second);
      expect(first.navigate).not.toHaveBeenCalled();
    } finally { a.cleanup(); }
  });

  it('reports only a fixed diagnostic for the selected post and only in debug builds', () => {
    const a = setup('combined', 'foryou', true);
    try {
      const missing = a.mount('');
      expect(a.api.diagnosis(missing.root, post)).toBe('entry-no-url');
      expect(a.api.diagnosis(missing.root, '/alice/status/654321')).toBe('entry-not-observed');
      const bound = a.mount('/bob/status/654321/mediaViewer', '654321');
      expect(a.api.diagnosis(bound.root, '/bob/status/654321')).toBe('entry-bound');
      a.mount(href);
      expect(a.api.diagnosis(missing.root, post)).toBe('entry-mixed');
      expect(setup('combined').api.diagnosis).toBeUndefined();
    } finally { a.cleanup(); }
  });

  it('leaves preview registration to its own controls', () => {
    const a = setup('combined','preview');
    expect(a.api).toBeUndefined();
  });

  it('scopes screen.width below theme.breakpoints.large only inside the verified tweet entry module', () => {
    const entryFixture = 'function(m,exports,load){var d=load(71),u=load(78);class Tweet{_isClientAllowedIntoImmersiveViewer=()=>!(d.A.get("screen").width>=u.A.theme.breakpoints.large)&&this.context.featureSwitches.isTrue("explore_relaunch_enable_immersive_web");_renderRichContent(){return{tweetMediaItemImmersiveViewerEntryPoint:()=>"/alice/status/123456/mediaviewer"}}}exports.measure=kind=>d.A.get(kind);exports.Tweet=Tweet}';
    let available = false;
    const location = new URL('https://x.com/home');
    const runtime: Record<string, any> = {
      location, URL,
      __atsumiXSurface: {sourceMediaViewerAvailable: () => available},
    };
    runtime.window = runtime; runtime.top = runtime;
    const context = vm.createContext(runtime);
    for (const script of [bridge, delivery]) {
      vm.runInContext(script.replace('__ATSUMI_X_SOURCE_COMPAT__', JSON.stringify({nonce:'test',feed:'foryou',diagnostics:true})), context);
    }
    const queue: any[] = [];
    runtime.webpackChunk_twitter_responsive_web = queue;
    const entryFn = vm.runInContext('(' + entryFixture + ')', context);
    queue.push([[505], {505373: entryFn}]);
    const dimensions = {get: vi.fn((kind: string) => ({width: kind === 'window' ? 1100 : 3440, height: 1440}))};
    const theme = {A: {theme: {breakpoints: {large: 800}}}};
    const exports: Record<string, any> = {};
    queue[0][1][505373]({}, exports, (id: number) => id === 71 ? {A: dimensions} : id === 78 ? theme : {});

    expect(exports.measure('screen').width).toBe(3440);
    available = true;
    expect(exports.measure('screen')).toEqual({width: 799, height: 1440});
    expect(exports.measure('window')).toEqual({width: 1100, height: 1440});
    expect(dimensions.get('screen').width).toBe(3440);
    expect(theme.A.theme.breakpoints.large).toBe(800);
    expect(runtime.__atsumiXVideoEntry.entryShape(entryFixture.replace('get("screen")', 'get("other")'))).toBeNull();
  });

  it('resolves verified non-promoted video entries when the route adapter is available while skipping ads and gifs', () => {
    const location = new URL('https://x.com/home');
    let available = true;
    const runtime: Record<string, any> = {
      location, URL,
      __atsumiXSurface: {sourceMediaViewerAvailable: () => available},
    };
    runtime.window = runtime; runtime.top = runtime;
    const context = vm.createContext(runtime);
    for (const script of [bridge, delivery]) {
      vm.runInContext(script.replace('__ATSUMI_X_SOURCE_COMPAT__', JSON.stringify({nonce:'test',feed:'foryou',diagnostics:true})), context);
    }
    const factory = (code: string) => vm.runInContext('(' + code + ')', context);
    const queue: any[] = [];
    runtime.webpackChunk_twitter_responsive_web = queue;
    queue.push([[113], {924: factory(publisher), 236: factory(player)}]);
    const modules = queue[0][1];
    const builder = {compact: vi.fn(() => ({kind:'compact'})), normal: vi.fn(() => ({kind:'desktop'}))};
    const jsx = (component: unknown, props: Record<string, any>) => ({component, props});
    const imports: Record<number, any> = {11:{jsx},17:{A:class {}},29:builder};
    const address: Record<string, any> = {}, controls: Record<string, any> = {};
    modules[236]({}, controls, (id: number) => imports[id]);
    modules[924]({}, address, (id: number) => imports[id]);

    const items = address.list([
      {tweetId:'123456', type:'video', authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
      {tweetId:'654321', type:'video', authorScreenName:'alice', promotedContent:{id:'ad'}, mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
      {tweetId:'777777', type:'animated_gif', authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
    ]).props.media as Record<string, any>[];
    expect(items[0]!.immersiveViewerUrl).toBe('/alice/status/123456/mediaviewer?mediaIndex=1');
    expect(items[1]!.immersiveViewerUrl).toBeUndefined();
    expect(items[2]!.immersiveViewerUrl).toBeUndefined();

    const single = address.list([
      {tweetId:'123456', type:'video', authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
    ]).props.media as Record<string, any>[];
    expect(single[0]!.immersiveViewerUrl).toBe('/alice/status/123456/mediaviewer');

    const root = document.createElement('div'), host = document.createElement('div'), button = document.createElement('button');
    root.dataset.testid = 'videoPlayer'; root.append(button); host.append(root); document.body.append(host);
    try {
      const navigate = vi.fn((url: string) => { location.href = new URL(url, location).href; });
      const rendered = new controls.Player({...single[0]!}, {push:navigate, location})._renderInlinePlayer();
      expect(rendered.props.children.props.uiStack.kind).toBe('desktop');
      host.setAttribute('data-atsumi-viewer-owner', '');
      rendered.props.ref(host);
      expect(runtime.__atsumiXVideoEntry.open({isTrusted:true}, root, button, post)).toBe(true);
      expect(navigate).toHaveBeenCalledExactlyOnceWith('/alice/status/123456/mediaviewer');
      rendered.props.ref(null);
    } finally {
      host.remove();
    }

    location.pathname = '/alice';
    location.href = 'https://x.com/alice';
    const onProfile = address.list([
      {tweetId:'123456', type:'video', authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
    ]).props.media as Record<string, any>[];
    expect(onProfile[0]!.immersiveViewerUrl).toBe('/alice/status/123456/mediaviewer?mode=profile');

    available = false;
    const disabled = address.list([
      {tweetId:'123456', type:'video', authorScreenName:'alice', mediaPermalink:undefined, getMediaSessionMetadata:()=>undefined},
    ]).props.media as Record<string, any>[];
    expect(disabled[0]!.immersiveViewerUrl).toBeUndefined();
  });
});
