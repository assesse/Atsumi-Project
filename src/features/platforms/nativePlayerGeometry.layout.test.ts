import { describe, expect, it } from 'vitest';
import { auditLayouts, layoutBrowser, type LayoutScene } from '../../test/offlineLayoutBrowser';

const css = `*{box-sizing:border-box}html,body{margin:0;width:100%;height:100%;overflow:hidden}
  .workspace{position:relative;margin-left:214px;width:900px;height:700px;overflow:hidden;background:#111}
  .fixed{position:fixed;left:0;top:0;width:1266px;height:782px;z-index:3}
  .detail{position:absolute;left:34px;top:176px;width:951px;height:539px;overflow:auto}
  #player{width:951px;height:539px;background:#148dcb}
  .cb{position:relative;margin-left:214px;margin-top:100px;width:400px;height:400px;overflow:hidden}
  .cb>.fixed{left:-180px;top:0;width:600px;height:300px}.cb #player{width:600px;height:300px}`;
const wideSamples = [{x:60,y:200},{x:250,y:200},{x:980,y:200}];
const containedSamples = [{x:60,y:120},{x:250,y:120},{x:620,y:120}];
const scenes: LayoutScene[] = [
  {name:'viewport-fixed-floating-detail',html:'<div class="workspace"><div class="fixed"><div class="detail" role="dialog" aria-modal="true"><div id="player"></div></div></div></div>',samples:wideSamples},
  {name:'viewport-fixed-slot-itself',html:'<div class="workspace"><div id="player" style="position:fixed;left:34px;top:176px"></div></div>',samples:wideSamples},
  {name:'internal-modal-scroll-is-real',html:'<div class="workspace"><div class="fixed"><div class="detail" style="width:500px"><div id="player"></div></div></div></div>',samples:wideSamples},
  {name:'ordinary-ancestor-clip-remains',html:'<div class="cb"><div id="player" style="position:relative;left:-180px"></div></div>',samples:containedSamples},
  ...['transform:translateZ(0)','contain:layout','contain:paint','will-change:transform','filter:blur(0px)','perspective:1000px','translate:0px','scale:1','rotate:0deg','backdrop-filter:blur(0px)','content-visibility:auto','will-change:filter','will-change:perspective','will-change:contain'].map((style,index)=>({name:`real-fixed-containing-block-${index}`,html:`<div class="cb" style="${style}"><div class="fixed"><div id="player"></div></div></div>`,samples:containedSamples})),
  {name:'intermediate-overflow-skipped-until-real-containing-block',html:'<div class="cb" style="transform:translateZ(0)"><div style="margin-left:100px;width:100px;height:100px;overflow:hidden"><div class="fixed"><div id="player"></div></div></div></div>',samples:containedSamples},
  {name:'nested-fixed-escapes-internal-scroll',html:'<div class="workspace"><div class="fixed"><div class="detail" style="width:500px"><div id="player" style="position:fixed;left:34px;top:176px"></div></div></div></div>',samples:wideSamples},
];
describe.skipIf(!layoutBrowser)('native floating clipping in a real offline browser',()=>{
  it('matches actual browser visibility, including containing-block counterexamples',async()=>{
    const results=await auditLayouts(scenes,css,1266,782,'native-fixed-geometry',true);
    expect(results).toHaveLength(scenes.length);
    expect(results.flatMap(result=>result.issues.map(issue=>`${result.name}: ${issue}`))).toEqual([]);
    expect(results[0]?.samples?.[0]).toMatchObject({browserVisible:true,nativeVisible:true});
    expect(results[3]?.samples?.[0]).toMatchObject({browserVisible:false,nativeVisible:false});
  },150000);
});
