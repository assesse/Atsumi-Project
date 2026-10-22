import { expect, it } from 'vitest';
import source from '../../../src-tauri/src/youtube/original_account.js?raw';
const vmName='node:vm';
const {runInNewContext}=await import(vmName) as {runInNewContext(source:string,context:Record<string,unknown>):unknown};
function observe({origin='https://www.youtube.com',pathname='/',search='',hash='',authenticated=true as unknown,editable=false,dialog=false,complete=true,iframe=false}={}){
 const window:Record<string,unknown>={ytcfg:{get:()=>authenticated}};window.top=iframe?{}:window;
 return runInNewContext(source,{window,location:{origin,pathname,search,hash},document:{readyState:complete?'complete':'loading',activeElement:{closest:()=>editable?{}:null,get value():never{throw new Error('No input reads');}},querySelectorAll:()=>dialog?[{getBoundingClientRect:()=>({width:20,height:20})}]:[]},getComputedStyle:()=>({display:'block',visibility:'visible'})});
}
it('reports fresh official authentication only after a non-interactive completed document',()=>{
 expect(observe()).toEqual({authenticated:true,interactive:false});expect(observe({authenticated:false})).toEqual({authenticated:false,interactive:false});expect(observe({authenticated:'unknown'})).toEqual({authenticated:null,interactive:true});expect(observe({authenticated:null})).toEqual({authenticated:null,interactive:true});
});
it('keeps challenge, consent, active input, incomplete and foreign documents open',()=>{
 for(const options of [{editable:true},{dialog:true},{complete:false}])expect(observe(options)).toEqual({authenticated:true,interactive:true});
 for(const options of [{origin:'https://accounts.google.com'},{origin:'https://www.youtube.com.evil.test'},{iframe:true},{pathname:'/signin'},{search:'?auth=1'},{hash:'#consent'}])expect(observe(options)).toEqual({authenticated:null,interactive:true});
 for(const forbidden of ['document.cookie','localStorage','fetch(','XMLHttpRequest','.value','.textContent'])expect(source).not.toContain(forbidden);
});
it('observes dialog dismissal and editable blur on the same completed homepage without changing the document',()=>{
 const location={origin:'https://www.youtube.com',pathname:'/',search:'',hash:''};
 let dialog=true,editable=false,authenticated=true;
 const window:Record<string,unknown>={ytcfg:{get:()=>authenticated}};window.top=window;
 const context={window,location,document:{readyState:'complete',activeElement:{closest:()=>editable?{}:null},querySelectorAll:()=>dialog?[{getBoundingClientRect:()=>({width:20,height:20})}]:[]},getComputedStyle:()=>({display:'block',visibility:'visible'})};
 expect(runInNewContext(source,context)).toEqual({authenticated:true,interactive:true});
 dialog=false;editable=true;expect(runInNewContext(source,context)).toEqual({authenticated:true,interactive:true});
 editable=false;expect(runInNewContext(source,context)).toEqual({authenticated:true,interactive:false});
 authenticated=false;expect(runInNewContext(source,context)).toEqual({authenticated:false,interactive:false});
 for(const forbidden of ['.blur(','.click(','.submit(','.reload(','setInterval(','setTimeout('])expect(source).not.toContain(forbidden);
});
