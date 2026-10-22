import { describe, expect, it } from 'vitest';
import source from '../../../src-tauri/src/soop/original_account.js?raw';
const vmName='node:vm';
const {runInNewContext}=await import(vmName) as {runInNewContext(source:string,context:Record<string,unknown>):unknown};
async function settle(){for(let i=0;i<400;i++)await Promise.resolve();}
type Options={url?:string;value?:unknown;fail?:boolean;nested?:boolean;length?:string|null;missing?:boolean;redirect?:boolean;chunks?:number;byteCount?:number;pending?:boolean};
function fixture(options:Options={}){
  let now=0,nextTimer=0,cancels=0,releases=0;
  const timers=new Map<number,{at:number;handler:()=>void}>(),listeners=new Map<string,()=>void>(),messages:string[]=[],requests:{endpoint:string;options:any}[]=[];
  const waiting:(()=>void)[]=[];
  const window:any={chrome:{webview:{postMessage:(message:string)=>messages.push(message)}},addEventListener:(name:string,handler:()=>void)=>listeners.set(name,handler)};
  window.top=options.nested?{}:window;
  const context={window,location:new URL(options.url??'https://www.sooplive.com/'),document:{addEventListener:(name:string,handler:()=>void)=>listeners.set(name,handler)},performance:{now:()=>now},Date,AbortController,TextDecoder,
    setTimeout:(handler:()=>void,delay:number)=>{const id=++nextTimer;timers.set(id,{handler,at:now+delay});return id;},clearTimeout:(id:number)=>timers.delete(id),
    fetch:async(endpoint:string,request:any)=>{
      requests.push({endpoint,options:request});
      if(options.pending)await new Promise<void>(resolve=>waiting.push(resolve));
      if(options.fail)throw Error('fixture');
      let reads=0;
      const payload=options.byteCount?new Uint8Array(options.byteCount):new TextEncoder().encode(JSON.stringify({CHANNEL:{IS_LOGIN:options.missing?undefined:('value'in options?options.value:1),CANARY:'never-export'}}));
      return {ok:true,url:options.redirect?'https://www.sooplive.com/':endpoint,headers:{get:()=>options.length??null},body:{getReader:()=>({read:async()=>reads++<(options.chunks??1)?{done:false,value:payload}:{done:true},cancel:async()=>{cancels++;},releaseLock:()=>{releases++;}})}};
    }};
  runInNewContext(source.replace('__ATSUMI_SOOP_ACCOUNT_NONCE__','"fixture-nonce"'),context);
  return {adapter:window.__atsumiSoopAccount,messages,requests,listeners,timers,waiting,get cancels(){return cancels;},get releases(){return releases;},
    async tick(){const next=[...timers].sort((a,b)=>a[1].at-b[1].at)[0];if(!next)return;timers.delete(next[0]);now=next[1].at;next[1].handler();await settle();}};
}
describe('SOOP current official home observation',()=>{
  it.each([[1,true],[-1,false]])('projects only explicit official state %s',async(value,authenticated)=>{
    const f=fixture({value});expect(f.adapter.configure(1)).toBe(true);await settle();
    expect(f.messages.map(value=>JSON.parse(value.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length)))).toEqual([{nonce:'fixture-nonce',epoch:1,authenticated}]);
    expect(f.messages.join('')).not.toMatch(/CANARY|never-export/);expect(f.requests).toHaveLength(1);
    expect(f.requests[0]!.endpoint).toMatch(/^https:\/\/afevent2\.sooplive\.com\/api\/get_private_info\.php\?_=[0-9]+$/);
    expect(f.requests[0]!.options).toMatchObject({credentials:'include',redirect:'error',cache:'no-store'});
    f.adapter.configure(1);await settle();expect(f.requests).toHaveLength(1);expect(f.releases).toBe(1);
  });
  it.each([true,false,'1','-1',0,null])('unknown enum %s never becomes false and retries at most three times',async value=>{
    const f=fixture({value});f.adapter.configure(1);await settle();for(let i=0;i<8;i++)await f.tick();
    expect(f.messages).toEqual([]);expect(f.requests).toHaveLength(3);expect(f.timers.size).toBe(0);
  });
  it.each([{fail:true},{length:'32769'},{length:'invalid'},{missing:true},{redirect:true},{byteCount:32769},{chunks:128}])('fails closed for unconfirmed response %j',async options=>{
    const f=fixture(options);f.adapter.configure(1);await settle();for(let i=0;i<8;i++)await f.tick();
    expect(f.messages).toEqual([]);expect(f.requests).toHaveLength(3);expect(f.timers.size).toBe(0);
    if(options.chunks||options.byteCount)expect(f.cancels).toBe(3);
  });
  it.each(['http://www.sooplive.com/','https://www.sooplive.com.evil.test/','https://www.sooplive.com/other','https://www.sooplive.com/?x=1','https://www.sooplive.com/#x','https://www.sooplive.com:444/'])('does not observe nonexact home %s',url=>{const f=fixture({url});expect(f.adapter).toBeUndefined();expect(f.requests).toEqual([]);});
  it('rejects nested documents and preserves login-host callback-only behavior',()=>{expect(fixture({nested:true}).adapter).toBeUndefined();const f=fixture({url:'https://login.sooplive.com/afreeca/login.php'});f.adapter.configure(1);expect(f.requests).toEqual([]);expect(f.messages).toEqual([]);});
  it('aborts on pagehide and cannot publish an old document reply',async()=>{const f=fixture({pending:true});f.adapter.configure(1);f.listeners.get('pagehide')!();f.waiting[0]!();await settle();expect(f.messages).toEqual([]);expect(f.timers.size).toBe(0);expect(f.adapter.configure(2)).toBe(false);});
  it('reconfiguring fences old responses and keeps a single current request',async()=>{const f=fixture({pending:true});f.adapter.configure(1);f.adapter.configure(2);expect(f.requests).toHaveLength(2);expect(f.requests[0]!.options.signal.aborted).toBe(true);f.waiting[0]!();f.waiting[1]!();await settle();expect(f.messages.map(value=>JSON.parse(value.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length)).epoch)).toEqual([2]);});
  it('aborts a five-second request without accepting its late successful response',async()=>{const f=fixture({pending:true});f.adapter.configure(1);await f.tick();expect(f.requests[0]!.options.signal.aborted).toBe(true);f.waiting[0]!();await settle();expect(f.messages).toEqual([]);});
});
