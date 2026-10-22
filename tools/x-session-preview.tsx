// Isolated production-shell fixture. No X network traffic, account, or native
// WebView is emulated as verified: opaque placeholders explicitly mark that limit.
import {useState} from 'react';
import {createRoot} from 'react-dom/client';
import {XWorkspace} from '../src/features/x/XWorkspace';
import type {XSurfaceApi,XSurfaceSnapshot} from '../src/features/x/api';
const accounts=[{id:'one',handle:'studio_sample'},{id:'two',handle:'photo_sample'}];
let active='one',generation=1,login=false;
let state:XSurfaceSnapshot={generation,feed:'foryou',state:'ready',preview:false,previewState:'empty',error:null};
const stats={opens:0,selections:0,suspensions:0,viewports:0,lastViewport:null as unknown};
const snapshot=()=>({open:login,status:'signedIn' as const,generation,active,accounts});
const api:XSurfaceApi={
  surface:async(feed,_order,_refresh)=>{stats.opens++;await new Promise(r=>setTimeout(r,400));state={...state,generation:++generation,feed,preview:false,previewState:'empty',state:'ready',error:null};return state;},
  surfaceStatus:async()=>state,viewport:async(_generation,viewport)=>{stats.viewports++;stats.lastViewport=viewport;},
  closePreview:async()=>{state={...state,generation:++generation,preview:false,previewState:'empty'};return state;},
  suspend:async()=>{stats.suspensions++;},
  account:async()=>{login=true;setTimeout(()=>{login=false;generation++;},1600);return snapshot();},
  status:async()=>snapshot(),select:async(id)=>{active=id;generation++;stats.selections++;return snapshot();},
};
function Fixture(){
  const [privacy,setPrivacy]=useState(false);
  Object.assign(window,{xFixture:{stats,privacy:setPrivacy,preview:(phase='ready')=>{state={...state,generation:++generation,preview:true,previewState:phase};},
    phase:(value:string)=>{state={...state,state:value};},error:(message:string)=>{state={...state,error:message};}}});
  return <XWorkspace api={api} active runtime="browser-mock" privacyMode={privacy} onNotice={console.warn}/>;
}
createRoot(document.getElementById('root')!).render(<Fixture/>);
