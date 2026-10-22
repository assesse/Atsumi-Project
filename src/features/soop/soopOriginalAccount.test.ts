import { describe, expect, it, vi } from 'vitest';
import source from '../../../src-tauri/src/soop/original_account.js?raw';
const vmName='node:vm';
const {runInNewContext}=await import(vmName) as {runInNewContext(source:string,context:Record<string,unknown>):unknown};
function fixture(origin='https://login.sooplive.com',nested=false,firstSucceeds=true){
  const sent:string[]=[];
  let ready=()=>{};
  const window:{[key:string]:any}={isLogin:false,chrome:{webview:{postMessage:(value:string)=>sent.push(value)}}};window.top=nested?{}:window;
  const result={},secondResult={};const login=vi.fn(function(this:unknown){window.isLogin=firstSucceeds;return result;});const logout=vi.fn(()=>{window.isLogin=false;});
  const secondLogin=vi.fn(function(this:unknown){window.isLogin=true;return secondResult;});
  window.onAfterLogin=login;window.onAfterSecondLogin=secondLogin;window.onAfterLogOut=logout;window.LogOut=vi.fn(()=>window.onAfterLogOut());
  runInNewContext(source.replace('__ATSUMI_SOOP_ACCOUNT_NONCE__','"account-nonce"'),{window,location:{origin},document:{addEventListener:(name:string,callback:()=>void)=>{if(name==='DOMContentLoaded')ready=callback;}}});
  return{window,sent,login,secondLogin,logout,result,secondResult,ready:()=>ready()};
}
describe('SOOP official account boolean observer',()=>{
  it('preserves official callback return, this and opaque arguments while emitting only boolean',()=>{
    const f=fixture();f.window.__atsumiSoopAccount.configure(3);
    const opaque=new Proxy({}, {get(){throw new Error('must not inspect callback data');}}), receiver={};
    expect(f.window.onAfterLogin.call(receiver,opaque)).toBe(f.result);expect(f.login.mock.contexts[0]).toBe(receiver);expect((f.login.mock.calls as unknown[][])[0]![0]===opaque).toBe(true);
    expect(f.sent.map(value=>JSON.parse(value.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length)))).toEqual([{nonce:'account-nonce',epoch:3,authenticated:true}]);
    f.window.__atsumiSoopAccount.logout();expect(f.window.LogOut).toHaveBeenCalledWith('https://www.sooplive.com/');
    expect(JSON.parse(f.sent[1]!.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length)).authenticated).toBe(false);
  });
  it('does not invent an account result merely from opening a page',()=>{const f=fixture();f.window.__atsumiSoopAccount.configure(1);expect(f.sent).toEqual([]);});
  it('observes successful second login after an incomplete first login without inspecting callback arguments',()=>{
    const f=fixture(undefined,false,false);f.window.__atsumiSoopAccount.configure(4);
    f.window.onAfterLogin();
    const opaque=new Proxy({}, {get(){throw new Error('must not inspect second-login data');}}),receiver={};
    expect(f.window.onAfterSecondLogin.call(receiver,opaque,false)).toBe(f.secondResult);
    expect(f.secondLogin.mock.contexts[0]).toBe(receiver);
    expect((f.secondLogin.mock.calls as unknown[][])[0]![0]===opaque).toBe(true);
    expect((f.secondLogin.mock.calls as unknown[][])[0]![1]).toBe(false);
    expect(f.sent.map(value=>JSON.parse(value.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length)))).toEqual([
      {nonce:'account-nonce',epoch:4,authenticated:false},{nonce:'account-nonce',epoch:4,authenticated:true},
    ]);
  });
  it('buffers an actual early callback until native configuration, and replays only that observation for a new epoch',()=>{
    const f=fixture();f.ready();f.window.onAfterLogin();expect(f.sent).toEqual([]);
    f.window.__atsumiSoopAccount.configure(1);expect(f.sent).toHaveLength(1);
    expect(JSON.parse(f.sent[0]!.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length))).toMatchObject({epoch:1,authenticated:true});
    f.window.__atsumiSoopAccount.configure(1);expect(f.sent).toHaveLength(1);
    f.window.isLogin=false; // A page variable change without an official callback is not evidence.
    f.window.__atsumiSoopAccount.configure(2);expect(f.sent).toHaveLength(2);
    expect(JSON.parse(f.sent[1]!.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length))).toMatchObject({epoch:2,authenticated:true});
    f.window.onAfterLogOut();expect(JSON.parse(f.sent[2]!.slice('ATSUMI_SOOP_ACCOUNT_V1:'.length))).toMatchObject({epoch:2,authenticated:false});
  });
  it('does not install in another platform or subframe',()=>{expect(fixture('https://chzzk.naver.com').window.__atsumiSoopAccount).toBeUndefined();expect(fixture(undefined,true).window.__atsumiSoopAccount).toBeUndefined();});
  it('does not read credentials, cookies, storage or account IDs; the only request is the bounded official home observation',()=>{for(const forbidden of ['document.cookie','localStorage','XMLHttpRequest','.RESULT','userId','loginId','password','setInterval'])expect(source).not.toContain(forbidden);expect(source.match(/fetch\(/g)).toHaveLength(1);expect(source).toContain('https://afevent2.sooplive.com/api/get_private_info.php?_=');});
});
