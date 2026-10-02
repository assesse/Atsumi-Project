window.addEventListener('DOMContentLoaded', async () => {
  const {invoke,transformCallback}=window.__TAURI_INTERNALS__;
  const listen=(event,handler)=>invoke('plugin:event|listen',{event,target:{kind:'Any'},handler:transformCallback(handler)});
  const waiting=new Map(); let sequence=0, errors=0;
  try {
    await listen('probe:body', async ({payload:event}) => {
      const pending=waiting.get(event.requestId); if (!pending) return;
      const t=event.outcome.delivery.thumbnail;
      try {
        const buffer=t.resourceToken ? await invoke('probe_read',{token:t.resourceToken}) : Uint8Array.from(t.bytes).buffer;
        if (!(buffer instanceof ArrayBuffer) || (t.byteLength && buffer.byteLength!==t.byteLength)) throw new Error('not raw binary');
        const url=URL.createObjectURL(new Blob([buffer],{type:t.contentType}));
        const img=new Image(); img.src=url; await img.decode();
        if(img.naturalWidth!==512) throw new Error('decode mismatch');
        img.src=''; URL.revokeObjectURL(url);
        if(t.resourceToken) await invoke('probe_release',{token:t.resourceToken});
        waiting.delete(event.requestId); pending.resolve();
      } catch(e) {errors++;pending.reject(e);}
    });
    for (const mode of ['legacy','binary','binary']) {
      await invoke('probe_sample',{phase:`${mode}-before`,elapsedMs:0});
      const start=performance.now();
      for(let round=0;round<16;round++) await Promise.all(Array.from({length:4},()=>new Promise((resolve,reject)=>{
        const id=`probe-${++sequence}`; waiting.set(id,{resolve,reject});
        invoke('probe_request',{id,mode}).catch(reject);
      })));
      await invoke('probe_sample',{phase:`${mode}-after-64`,elapsedMs:performance.now()-start});
    }
    await invoke('probe_done',{ok:errors===0 && waiting.size===0});
  } catch(e) {console.error(e);await invoke('probe_done',{ok:false});}
},{once:true});
