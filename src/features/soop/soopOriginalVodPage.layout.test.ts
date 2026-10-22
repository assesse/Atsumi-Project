import { describe, expect, it } from 'vitest';
import script from '../../../src-tauri/src/soop/original_vod_page.js?raw';
import { auditLayouts, layoutBrowser, type LayoutScene } from '../../test/offlineLayoutBrowser';
import type { NativeViewport } from '../platforms/nativePlayerGeometry';

// Exact public cascade relevant to the defect, fetched2026-10-15 from
// https://res.sooplive.com/css/global/player/player.css. No network in this test.
const officialCss = `.embeded_mode #webplayer #player{flex:1 0 auto;max-height:none!important;min-width:auto;height:100vh!important;width:auto}
  #player{position:relative;overflow:hidden;background:#000;margin:0!important;max-height:calc(100vh - 180px);min-width:500px}
  #player .player_size{overflow:hidden;padding-top:56.25%;max-height:calc(100vh - 180px)}
  #player .af_video,#player #videoLayer{position:absolute!important;top:0;left:0;right:0;bottom:0;z-index:1;width:100%!important;height:100%!important;background-color:#000;padding:0!important}
  #player .player_ctrlBox{position:absolute;bottom:-160px;left:0;right:0;height:79px;z-index:20}
  #player.mouseover .player_ctrlBox{bottom:0}`;
const viewports=[{x:50,y:112,width:900,height:507},{x:30,y:90,width:640,height:360},{x:0,y:0,width:1266,height:782}];
describe.skipIf(!layoutBrowser)('SOOP original VOD embed height cascade in offline Chrome',()=>{
  it('uses the requested height regardless of stylesheet load order without forcing controls visible',async()=>{
    // Execute the exact static stylesheet emitted by the production adapter,
    // not a rewritten geometry rule. Fail if its form changes unexpectedly.
    const adapterCss=[...script.matchAll(/style\.textContent \+?= `([^`]+)`;/g)].map(match=>match[1]).join('');
    expect(adapterCss).toBeTruthy();
    const scenes:LayoutScene[]=[];
    for(const [index,rect] of viewports.entries()) for(const order of ['official-first','official-last']) for(const hovered of [false,true]) for(const rootClass of ['', 'embeded_mode']){
      const variables=`:root{--atsumi-vod-player-x:${rect.x}px;--atsumi-vod-player-y:${rect.y}px;--atsumi-vod-player-w:${rect.width}px;--atsumi-vod-player-h:${rect.height}px;--atsumi-vod-player-display:block;--atsumi-vod-chat-display:none}`;
      scenes.push({name:`${index}-${order}-${rootClass?'root':'descendant'}-${hovered?'hover':'hidden'}`,rootClass,html:`<style>${variables}${order==='official-first'?officialCss+adapterCss:adapterCss+officialCss}</style><div class="${rootClass?'':'embeded_mode'}"><div id="webplayer"><div id="player_area"><div id="player" class="${hovered?'mouseover':''}"><div class="player_size"></div><div id="videoLayer"><video id="video" class="af_video"></video></div><div class="player_ctrlBox"><button>original control</button></div></div></div></div></div>`});
    }
    const results=await auditLayouts(scenes,'html,body{margin:0;width:100%;height:100%}',1266,782,'soop-vod-embed-height',true);
    expect(results).toHaveLength(24);
    for(const result of results){
      const rect=viewports[Number(result.name.split('-')[0])]!;
      expect((result.geometry as NativeViewport).player,result.name).toEqual(rect);
      expect(result.nodes?.video,result.name).toEqual(rect);
      const bottom=(result.nodes?.controls?.y??0)+(result.nodes?.controls?.height??0);
      expect(bottom,result.name).toBe(rect.y+rect.height+(result.name.endsWith('hover')?0:160));
    }
  },150000);
});
