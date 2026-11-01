// @ts-expect-error Node's isolated VM is available in Vitest, outside the browser app's type scope.
import vm from 'node:vm';
import { describe, expect, it } from 'vitest';
import script from '../../../src-tauri/src/x_surface_source_delivery.js?raw';

const key = 'webpackChunk_twitter_responsive_web';
const apiKey = '__atsumiXSourceCompat';
const factory = new Function('return function(){class V{_renderBlurredMediaTombstone=()=>{let{displaySensitiveMedia:t}=this.props;return "rweb_age_assurance_flow_enabled"}}}')();
const original = () => [[123], { 456: factory }];
const fixed = () => [[123], { 456: factory }];
function setup(hostname = 'x.com', feed = 'preview') {
  const runtime: Record<string, any> = {
    location: { protocol: 'https:', hostname, port: '' },
    setTimeout() { throw Error('No source polling or packet delay allowed'); },
  };
  runtime.window = runtime; runtime.self = runtime; runtime.top = runtime;
  vm.runInContext(script.replace('__ATSUMI_X_SOURCE_COMPAT__', JSON.stringify({nonce:'owned',feed})), vm.createContext(runtime));
  runtime[key] = [];
  return { runtime, queue: runtime[key] as any[], api: runtime[apiKey] };
}

describe('public source compatibility handoff', () => {
  const routeFactory=new Function('return function(){return ["explore_relaunch_enable_immersive_web","explore_relaunch_enable_video_carousels_immersive_viewer","immersiveMediaViewer"]}')();
  const route=()=>[[234],{567:routeFactory}];
  const layoutFactory=new Function('return function(){return {ImmersiveMediaViewerScreen:()=>null,testID:"modalContainer"}}')();
  it('delivers both initial chunks in either order and retains the handoff for the later lazy viewer',()=>{
    for(const order of [['media','viewer'],['viewer','media']]){
      const {runtime,queue,api}=setup('x.com','foryou');
      for(const kind of order){const packet=kind==='media'?fixed():route();expect(api.deliver('owned',()=>queue.push(packet),kind)).toBe(true);queue.push(kind==='media'?original():route());expect(queue.at(-1)).toBe(packet);}
      expect(queue).toHaveLength(2);expect(runtime[apiKey]).toBe(api);
      const lazy=[[345],{678:layoutFactory}];queue.push(lazy);
      expect(queue).toHaveLength(3);expect(queue.at(-1)).toBe(lazy);expect(runtime[apiKey]).toBeUndefined();
    }
  });
  it('does not let a missing or failed chunk suppress the other original registration',()=>{
    const {queue,api}=setup('x.com','foryou');
    expect(api.deliver('owned',()=>{throw new Error('route fetch failed')},'viewer')).toBe(false);
    const before=route();queue.push(before);expect(queue).toEqual([before]);
    const after=fixed();expect(api.deliver('owned',()=>queue.push(after),'media')).toBe(true);queue.push(original());expect(queue).toEqual([before,after]);
  });
  it('does not admit a late route correction after its original source has registered',()=>{
    const {queue,api}=setup('x.com','foryou');const before=route();queue.push(before);
    expect(api.deliver('owned',()=>queue.push(route()),'viewer')).toBe(false);expect(queue).toEqual([before]);
    expect(api.status().kinds.viewer).toEqual({delivered:false,originalSeen:true,closed:true});
  });
  it('never registers the feed route correction in preview documents',()=>{
    const {queue,api}=setup();const before=route();queue.push(before);
    expect(api.deliver('owned',()=>queue.push(route()),'viewer')).toBe(false);expect(queue).toEqual([before]);
  });
  it('never holds an original network packet past its onload and refuses a late correction', () => {
    const { runtime, queue, api } = setup();
    const before = original(), after = fixed();
    queue.push(before);
    expect(queue).toEqual([before]);
    expect(api.deliver('owned', () => queue.push(after))).toBe(false);
    expect(queue).toEqual([before]);
    expect(runtime[apiKey]).toBeUndefined();
    expect(Object.getOwnPropertyDescriptor(queue, 'push')?.get).toBeUndefined();
  });

  it('keeps the correction when the native compilation beats the original network script', () => {
    const { runtime, queue, api } = setup();
    const after = fixed();
    expect(api.deliver('owned', () => queue.push(after))).toBe(true);
    queue.push(original());
    expect(queue).toEqual([after]);
    expect(runtime[apiKey]).toBeUndefined();
  });

  it('preserves a webpack bootstrap push replacement without recursion or duplicate registration', () => {
    const { queue, api } = setup();
    const parent = queue.push.bind(queue), registered: unknown[] = [];
    queue.push = (packet: any) => { registered.push(packet); return parent(packet); };
    const after = fixed();
    api.deliver('owned', () => queue.push(after));
    queue.push(original());
    expect(registered).toEqual([after]);
    expect(queue).toEqual([after]);
    queue.push([[999], { 1: () => undefined }]);
    expect(registered).toHaveLength(2);
  });

  it('passes unrelated code immediately and preserves the original after a native failure', () => {
    const { runtime, queue, api } = setup();
    const other = [[999], { 1: () => undefined }], before = original();
    queue.push(other);
    expect(queue).toEqual([other]);
    api.release('owned'); queue.push(before);
    expect(queue).toEqual([other, before]);
    expect(runtime[apiKey]).toBeUndefined();
    let called = false;
    expect(api.deliver('owned', () => { called = true; })).toBe(false);
    expect(called).toBe(false);
  });

  it('refuses a foreign delivery token and falls back safely if native compilation fails', () => {
    const { runtime, queue, api } = setup();
    const before = original();
    let called = false;
    expect(api.deliver('other', () => { called = true; })).toBe(false);
    api.release('other');
    expect(called).toBe(false); expect(queue).toHaveLength(0);
    expect(api.deliver('owned', () => { throw Error('compile failed'); })).toBe(false);
    queue.push(before);
    expect(queue).toEqual([before]); expect(runtime[apiKey]).toBeUndefined();
  });

  it('does not install outside the original X document', () => {
    const { api } = setup('x.com.evil.test');
    expect(api).toBeUndefined();
  });

  it('does not report success or eat the original if source registration throws', () => {
    const { queue, api } = setup();
    const parent = queue.push.bind(queue), after = fixed(), before = original();
    queue.push = (packet: any) => { if (packet === after) throw TypeError('registration failed'); return parent(packet); };
    expect(api.deliver('owned', () => queue.push(after))).toBe(false);
    expect(api.status().errorClass).toBe('TypeError');
    queue.push(before);
    expect(queue).toEqual([before]);
  });

  it('preserves a source replacement of the public queue and restores the original push', () => {
    const { runtime, queue } = setup();
    const replacement: unknown[] = [];
    runtime[key] = replacement;
    expect(runtime[key]).toBe(replacement);
    expect(queue.push).toBe(Array.prototype.push);
    expect(runtime[apiKey]).toBeUndefined();
  });
});
