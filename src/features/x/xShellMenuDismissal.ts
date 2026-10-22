/** Native child-webview events never bubble into this app-shell document. */
export function bindXShellMenuDismissal(target:Window,current:()=>{generation:number;dismiss:(generation:number)=>Promise<void>}|null){
  let disposed=false,pending=false;
  const dismiss=(event:PointerEvent)=>{
    if(disposed||!event.isTrusted||event.isPrimary===false||![0,2].includes(event.button)||pending)return;
    const state=current();if(!state)return;
    pending=true;
    // No synthetic page gesture, preventDefault, propagation stop or focus change.
    // One in-flight dismissal is sufficient for all retained native documents.
    void Promise.resolve().then(()=>{if(!disposed)return state.dismiss(state.generation);})
      .catch(()=>{}).finally(()=>{pending=false;});
  };
  target.addEventListener("pointerdown",dismiss,true);
  return()=>{disposed=true;target.removeEventListener("pointerdown",dismiss,true);};
}
