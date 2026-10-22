import { afterEach,expect,it,vi } from "vitest";
import { bindXShellMenuDismissal } from "./xShellMenuDismissal";
afterEach(()=>vi.restoreAllMocks());
function fixture(){
  let handler!:EventListener;
  const add=vi.spyOn(window,"addEventListener").mockImplementation((type,listener)=>{if(type==="pointerdown")handler=listener as EventListener;});
  const remove=vi.spyOn(window,"removeEventListener");
  const dismiss=vi.fn().mockResolvedValue(undefined);
  let state:{generation:number;dismiss:typeof dismiss}|null={generation:4,dismiss};
  const stop=bindXShellMenuDismissal(window,()=>state);
  const send=(extra:Partial<PointerEvent>={})=>{const event={isTrusted:true,isPrimary:true,button:0,preventDefault:vi.fn(),stopPropagation:vi.fn(),...extra};handler(event as unknown as PointerEvent);return event;};
  return {add,remove,dismiss,stop,send,set:(value:typeof state)=>{state=value;}};
}
it("dismisses all native menus on shell pointer without synthetic clicks, interception or focus change",async()=>{
  const f=fixture(),target=document.createElement("button");document.body.append(target);target.focus();
  const click=vi.spyOn(target,"click");
  try{
    const event=f.send();await Promise.resolve();await Promise.resolve();
    expect(f.dismiss).toHaveBeenCalledExactlyOnceWith(4);expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();expect(click).not.toHaveBeenCalled();expect(document.activeElement).toBe(target);
    expect(f.add).toHaveBeenCalledWith("pointerdown",expect.any(Function),true);
  }finally{f.stop();target.remove();}
});
it("coalesces rapid shell presses while dismissal is in flight and accepts current generations later",async()=>{
  const f=fixture();let release!:()=>void;f.dismiss.mockImplementationOnce(()=>new Promise<void>(resolve=>{release=resolve;}));
  f.send();f.send();f.send({button:2});await Promise.resolve();expect(f.dismiss).toHaveBeenCalledOnce();
  release();await new Promise(resolve=>setTimeout(resolve,0));f.set({generation:9,dismiss:f.dismiss});f.send({button:2});await Promise.resolve();
  expect(f.dismiss).toHaveBeenLastCalledWith(9);f.stop();
});
it("ignores synthetic, non-primary, middle, hidden/private or disposed events",async()=>{
  const f=fixture();
  f.send({isTrusted:false});f.send({isPrimary:false});f.send({button:1});f.set(null);f.send();f.stop();f.send();
  await Promise.resolve();expect(f.dismiss).not.toHaveBeenCalled();expect(f.remove).toHaveBeenCalledWith("pointerdown",expect.any(Function),true);
});
it("does not send a queued command after listener teardown and safely absorbs stale-generation rejection",async()=>{
  const f=fixture();f.send();f.stop();await Promise.resolve();expect(f.dismiss).not.toHaveBeenCalled();
  const next=fixture();next.dismiss.mockRejectedValueOnce(new Error("stale generation"));next.send();
  await new Promise(resolve=>setTimeout(resolve,0));next.send();await Promise.resolve();expect(next.dismiss).toHaveBeenCalledTimes(2);next.stop();
});
