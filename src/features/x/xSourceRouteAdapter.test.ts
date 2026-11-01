// @ts-expect-error Isolated Node VM, outside browser product typings.
import vm from "node:vm";
import { expect, it, vi } from "vitest";
import { act, createElement, useEffect } from "react";
import { createRoot } from "react-dom/client";
import adapter from "../../../src-tauri/src/x_surface_route_adapter.js?raw";
import delivery from "../../../src-tauri/src/x_surface_source_delivery.js?raw";
const key="__atsumiXRouteAdapter";
const factory=`function(m,exports,r){var j=r(77),g=r(88),modal=r(66);const Viewer='original-viewer',size={full:'full'};const route=e=>(0,j.jsx)(g.A,{...e,component:Viewer,featureSwitchAnyOf:["explore_relaunch_enable_immersive_web","explore_relaunch_enable_video_carousels_immersive_viewer"],match:e.match,withoutScribeImpression:!0});function registration(){return(0,j.jsx)(modal.A,{component:route,disableAnimation:!0,exact:!0,modalSize:size.full,path:'/author/status/123456/mediaViewer'},"immersiveMediaViewer")};exports.registration=registration;exports.otherModal=(key,more,component=modal.A)=>(0,j.jsx)(component,{component:route,disableAnimation:!0,exact:!0,modalSize:size.full,path:'/author/status/123456/mediaViewer',...more},key);exports.render=route;exports.normal=e=>(0,j.jsx)(g.A,{...e,component:Viewer,featureSwitchAnyOf:["account_age_gate"],match:e.match});}`;
const layoutFactory='function(m,exports,r){var d=r(99),theme=r(101);function Screen(){const{width:w}=d.A.get("screen");if(w>=theme.A.theme.breakpoints.large)return void nav.push(`/${author}/status/${post}`);set(w>=theme.A.theme.breakpoints.large),nav.replace("source");}const published={ImmersiveMediaViewerScreen:()=>Screen,default:()=>Screen,testID:"modalContainer"};exports.measure=value=>d.A.get(value);exports.other=r(100);exports.render=Screen;}';
function setup(feed="foryou",host="x.com"){
  let available=false;
  const registered=vi.fn((nonce:string,next=true)=>{if(nonce==='owned')available=next;});
  const runtime:Record<string,any>={location:{protocol:'https:',hostname:host,port:''},__atsumiXSurface:{registerSourceMediaViewer:registered,sourceMediaViewerAvailable:()=>available}};
  runtime.window=runtime;runtime.top=runtime;
  const context=vm.createContext(runtime),config=JSON.stringify({nonce:'owned',feed,diagnostics:true});
  vm.runInContext(adapter.replace('__ATSUMI_X_SOURCE_COMPAT__',config),context);
  const original=vm.runInContext(`(${factory})`,context),gate=()=>null,jsx=vi.fn((component:any,props:any)=>({component,props}));
  const modal=Object.assign(()=>null,{defaultProps:{shouldAlwaysDisplayModal:()=>false,shouldRenderAsModal:()=>true}});
  const imports=(id:number)=>id===77?{jsx}:id===88?{A:gate}:id===66?{A:modal}:{};
  const packet=():[number[],Record<number,(...args:any[])=>any>]=>[[123],{456:original}];
  return{runtime,context,config,original,gate,modal,jsx,imports,packet,registered,api:runtime[key]};
}
it('adapts only the exact presentation wrapper while keeping the original viewer, source props and other gates',()=>{
  const a=setup(),packet=a.packet(),changed=a.api.adapt('owned',packet),exports:Record<string,any>={};
  expect(changed).not.toBe(packet);expect(packet[1][456]).toBe(a.original);
  changed[1][456]({},exports,a.imports);expect(a.registered).toHaveBeenCalledWith('owned');
  const rendered=exports.render({match:'source-match',history:'source-history'});
  expect(rendered).toEqual({component:'original-viewer',props:{match:'source-match',history:'source-history',withoutScribeImpression:true}});
  expect(exports.normal({match:'security'}).component).toBe(a.gate);
  expect(a.jsx).toHaveBeenCalledTimes(2);
});
it('requires original presentation imports and keeps the original gate if the JSX import cannot be verified',()=>{
  const a=setup(),changed=a.api.adapt('owned',a.packet()),exports:Record<string,any>={};
  changed[1][456]({},exports,(id:number)=>id===77?{jsx:undefined}:a.imports(id));expect(a.registered).not.toHaveBeenCalled();
  expect(a.runtime.__atsumiXSurface.sourceMediaViewerAvailable()).toBe(false);
});
it('refuses preview, foreign hosts/tokens, ambiguous factories and changed gates',()=>{
  expect(setup('preview').api).toBeUndefined();expect(setup('foryou','x.com.evil.test').api).toBeUndefined();
  const a=setup();expect(a.api.adapt('foreign',a.packet())).toBeNull();
  expect(a.api.adapt('owned',[[123],{456:a.original,789:a.original}])).toBeNull();
  for(const code of [factory.replace('explore_relaunch_enable_immersive_web','subscriptions_paid_gate'),factory.replace('match:e.match','match:other.match'),factory.replace('j=r(77)','j=unverified(77)'),factory.replace('component:route,','component:other,'),factory.replace('/status/','/unsupported/')])expect(a.api.shape(code)).toBeNull();
});
it('retains require properties and source registration without waiting for another network request',()=>{
  const a=setup();vm.runInContext(delivery.replace('__ATSUMI_X_SOURCE_COMPAT__',a.config),a.context);a.runtime.webpackChunk_twitter_responsive_web=[];
  const queue=a.runtime.webpackChunk_twitter_responsive_web,registered:unknown[]=[],exports:Record<string,any>={};
  queue.push=(packet:any)=>{registered.push(packet);packet[1][456]({},exports,a.imports);return 1;};
  const source=a.packet();queue.push(source);
  expect(registered).toHaveLength(1);expect(exports.render({match:'source'}).component).toBe('original-viewer');
  expect(a.runtime.__atsumiXSourceCompat.status().kinds.viewer).toEqual({delivered:true,originalSeen:true,closed:true});
  expect(source[1][456]).toBe(a.original);
});
it('revokes the optional entry if original source module initialization fails',()=>{
  const a=setup();const failed=vm.runInContext(`(${factory.replace('exports.render=route;','throw new Error("source failure");exports.render=route;')})`,a.context);
  const packet=a.api.adapt('owned',[[123],{456:failed}]);expect(()=>packet[1][456]({}, {},a.imports)).toThrow('source failure');
  expect(a.runtime.__atsumiXSurface.sourceMediaViewerAvailable()).toBe(false);expect(a.registered).toHaveBeenLastCalledWith('owned',false);
});
it('uses only bounded static shape inspection and no dynamic evaluation or source state access',()=>{
  expect(adapter).not.toMatch(/\beval\s*\(|new Function|document\.cookie|localStorage|\.play\s*\(|\.currentTime\s*=|require\.c|__react/);
  const a=setup();expect(a.api.shape('x'.repeat(4*1024*1024+1))).toBeNull();
});
it('measures only the verified immersive factory against its original pane viewport without changing global dimensions',()=>{
  const a=setup();a.registered('owned');
  const dimensions={get:vi.fn((name:string)=>({width:name==='window'?560:1600,height:900})),marker:'original'},other={A:dimensions};
  const original=vm.runInContext(`(${layoutFactory})`,a.context),exports:Record<string,any>={};
  const changed=a.api.adapt('owned',[[345],{678:original}],'layout');
  changed[1][678]({},exports,(id:number)=>id===99?{A:dimensions}:id===101?{A:{theme:{breakpoints:{large:1000}}}}:other);
  expect(exports.measure('screen').width).toBe(560);expect(exports.measure('window').width).toBe(560);
  expect(exports.measure('other').width).toBe(1600);expect(exports.other).toBe(other);
  expect(dimensions.get('screen').width).toBe(1600);
  a.registered('owned',false);expect(exports.measure('screen').width).toBe(1600);
});
it('preserves lazy original registration after both initial packets and revokes optional entry on layout drift',()=>{
  const a=setup();vm.runInContext(delivery.replace('__ATSUMI_X_SOURCE_COMPAT__',a.config),a.context);a.runtime.webpackChunk_twitter_responsive_web=[];
  const queue=a.runtime.webpackChunk_twitter_responsive_web,registered:any[]=[],exports:Record<string,any>={};
  const dimensions={get:(name:string)=>({width:name==='window'?560:1600})};
  queue.push=(packet:any)=>{registered.push(packet);for(const [id,fn] of Object.entries(packet[1]))(fn as Function)({},id==='678'?exports:{},id==='678'?(key:number)=>key===99?{A:dimensions}:key===101?{A:{theme:{breakpoints:{large:1000}}}}:{}:a.imports);return registered.length;};
  const api=a.runtime.__atsumiXSourceCompat;
  const media=vm.runInContext('(function(){const _renderBlurredMediaTombstone=()=>{return {displaySensitiveMedia:true,rweb_age_assurance_flow_enabled:true}};})',a.context);
  queue.push(a.packet());queue.push([[123],{456:media}]);
  expect(api.status().kinds.layout.closed).toBe(false);
  const original=vm.runInContext(`(${layoutFactory})`,a.context);queue.push([[345],{678:original}]);
  expect(registered).toHaveLength(3);expect(exports.measure('screen').width).toBe(560);expect(a.runtime.__atsumiXSourceCompat).toBeUndefined();
  for(const bad of [layoutFactory.replace('get("screen")','get("other")'),layoutFactory.replace('breakpoints.large','breakpoints.unknown')])expect(a.api.layoutShape(bad)).toBeNull();
  const b=setup();b.registered('owned');vm.runInContext(delivery.replace('__ATSUMI_X_SOURCE_COMPAT__',b.config),b.context);b.runtime.webpackChunk_twitter_responsive_web=[];
  const drift=vm.runInContext(`(${layoutFactory.replace('get("screen")','get("other")')})`,b.context);
  const packet=[[345],{678:drift}];b.runtime.webpackChunk_twitter_responsive_web.push(packet);
  expect(b.runtime.webpackChunk_twitter_responsive_web[0]).toBe(packet);expect(b.runtime.__atsumiXSurface.sourceMediaViewerAvailable()).toBe(false);
});
it('keeps the original immersive presentation in a wide pane without changing its actual window or theme geometry',()=>{
  const a=setup();a.registered('owned');const dimensions={get:(name:string)=>({width:name==='window'?1300:1920,height:900})},theme={A:{theme:{breakpoints:{large:1000}}}};
  const original=vm.runInContext(`(${layoutFactory})`,a.context),exports:Record<string,any>={};
  a.api.adapt('owned',[[345],{678:original}],'layout')[1][678]({},exports,(id:number)=>id===99?{A:dimensions}:id===101?theme:{});
  expect(exports.measure('screen')).toEqual({width:999,height:900});expect(exports.measure('window')).toEqual({width:1300,height:900});
  expect(dimensions.get('screen').width).toBe(1920);expect(theme.A.theme.breakpoints.large).toBe(1000);
});
it('refuses an unverified theme import and revokes optional entry for an invalid original breakpoint',()=>{
  const a=setup();expect(a.api.layoutShape(layoutFactory.replace('theme=r(101)','theme=foreign(101)'))).toBeNull();
  a.registered('owned');const original=vm.runInContext(`(${layoutFactory})`,a.context),exports:Record<string,any>={};
  a.api.adapt('owned',[[345],{678:original}],'layout')[1][678]({},exports,(id:number)=>id===99?{A:{get:()=>({width:1920})}}:id===101?{A:{theme:{breakpoints:{large:NaN}}}}:{});
  expect(a.runtime.__atsumiXSurface.sourceMediaViewerAvailable()).toBe(false);expect(exports.measure('screen').width).toBe(1920);
});
it('keeps one original player subtree through the source routers narrow and wide presentation boundary',async()=>{
  const a=setup(),exports:Record<string,any>={};a.api.adapt('owned',a.packet())[1][456]({},exports,a.imports);
  const route=exports.registration(),host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  let mounts=0,unmounts=0;
  function OriginalPlayer(){useEffect(()=>{mounts++;return()=>{unmounts++;};},[]);return createElement('span',{'data-source-player':'original'});}
  // Reduced original router135438: its L changes the parent tree, remounting
  // the original media. Actual DOM trace lost paused8.97s at this boundary.
  const display=(width:number)=>{const modal=!!route&&(width>=700||false)||!!route.props.shouldAlwaysDisplayModal?.({},{});const player=createElement(OriginalPlayer);return modal?createElement('section',{role:'dialog'},player):player;};
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
  try{
    await act(async()=>root.render(display(590)));const first=host.querySelector('[data-source-player]');
    for(const width of [1062,700,590]){await act(async()=>root.render(display(width)));expect(host.querySelector('[data-source-player]')).toBe(first);}
    expect(mounts).toBe(1);expect(unmounts).toBe(0);
  }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
it('pins only the original immersive route modal presentation and leaves other routes and explicit source props intact',()=>{
  const a=setup(),exports:Record<string,any>={};a.api.adapt('owned',a.packet())[1][456]({},exports,a.imports);
  const route=exports.registration();expect(route.component).toBe(a.modal);expect(route.props.shouldAlwaysDisplayModal({},{})).toBe(true);
  expect(route.props.path).toBe('/author/status/123456/mediaViewer');expect(a.modal.defaultProps.shouldAlwaysDisplayModal()).toBe(false);
  expect(exports.otherModal('photo',{}).props.shouldAlwaysDisplayModal).toBeUndefined();
  const explicit=()=>false;expect(exports.otherModal('immersiveMediaViewer',{shouldAlwaysDisplayModal:explicit}).props.shouldAlwaysDisplayModal).toBe(explicit);
  a.registered('owned',false);expect(exports.registration().props.shouldAlwaysDisplayModal).toBeUndefined();
});
it('refuses modal import or default predicate drift before granting the optional original entry',()=>{
  const a=setup();expect(a.api.shape(factory.replace('modal=r(66)','modal=foreign(66)'))).toBeNull();
  const exports:Record<string,any>={};a.api.adapt('owned',a.packet())[1][456]({},exports,(id:number)=>id===66?{A:()=>null}:a.imports(id));
  expect(a.runtime.__atsumiXSurface.sourceMediaViewerAvailable()).toBe(false);expect(exports.render({match:'source'}).component).toBe(a.gate);
});
